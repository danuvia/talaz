#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, path::PathBuf, process::Command};

mod device;
mod play;
mod runtime;

fn home() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?))
}

/// Everything Talaz keeps: the runtime, the game versions, the game's own data and the sign-in
pub fn root() -> Option<PathBuf> {
    Some(home()?.join(if cfg!(target_os = "macos") { "Library/Application Support/Talaz" } else { ".local/share/talaz" }))
}

/// Folders that hold game versions: Talaz's own, then mcpelauncher's so versions downloaded there still show up
fn version_dirs() -> Vec<PathBuf> {
    let theirs = home().map(|home| home.join(if cfg!(target_os = "macos") { "Library/Application Support/mcpelauncher" } else { ".local/share/mcpelauncher" }));
    root().into_iter().chain(theirs).map(|dir| dir.join("versions")).collect()
}

/// The mod, built before the launcher and carried inside it
#[cfg(target_arch = "aarch64")]
const MOD: &[u8] = include_bytes!("../../../mod/build/arm64/libtalaz_bedrock.so");
#[cfg(target_arch = "x86_64")]
const MOD: &[u8] = include_bytes!("../../../mod/build/x86_64/libtalaz_bedrock.so");

/// Where the game loads the mod from. The mod keeps its settings next to itself
fn mods_dir() -> Option<PathBuf> {
    Some(root()?.join("mod"))
}

/// Writes the mod out when it is missing or from another build of Talaz
fn place_mod() -> Result<PathBuf, String> {
    let dir = mods_dir().ok_or("No home folder")?;
    let lib = dir.join("libtalaz_bedrock.so");
    if std::fs::read(&lib).ok().as_deref() != Some(MOD) {
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        // a running game still has the old file open, so swap it instead of writing into it
        let part = dir.join("libtalaz_bedrock.so.part");
        std::fs::write(&part, MOD).and_then(|_| std::fs::rename(&part, &lib)).map_err(|err| err.to_string())?;
    }
    Ok(dir)
}

fn version_key(version: &str) -> Vec<u32> {
    version.split('.').filter_map(|part| part.parse().ok()).collect()
}

/// Installed game versions and where they are, newest first
fn installed() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for dir in version_dirs() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            // a .part folder is a download that did not finish
            if entry.path().is_dir() && !name.ends_with(".part") && !found.iter().any(|(known, _)| *known == name) {
                found.push((name, entry.path()));
            }
        }
    }
    found.sort_by_key(|(name, _)| std::cmp::Reverse(version_key(name)));
    found
}

// ponytail: the mod names live here and in mod/src/mods.cpp. Have the mod write its own list once there are many
const MODS: &[(&str, &str, &str)] = &[
    ("zoom", "Zoom", "Hold C to zoom in, scroll to zoom further"),
    ("fps", "FPS", "Frames per second on screen"),
    ("cps", "CPS", "Clicks per second"),
    ("keystrokes", "Keystrokes", "Shows the keys you press"),
    ("clock", "Clock", "The real time, so you know when to stop"),
    ("session", "Session", "How long you have been playing"),
    ("fpslimit", "FPS Limit", "Caps the frame rate to keep the computer cool and quiet"),
];

/// The file the mod keeps the active profile's settings in, next to its library
fn profile_file() -> Option<PathBuf> {
    let base = mods_dir()?.join("libtalaz_bedrock");
    let global = std::fs::read_to_string(base.with_extension("ini")).unwrap_or_default();
    let profile = global.lines().find_map(|line| line.strip_prefix("profile=")).unwrap_or("Default Profile");
    Some(base.with_extension(format!("profile.{profile}.ini")))
}

fn ini_get(text: &str, key: &str) -> Option<f32> {
    text.lines().filter_map(|line| line.split_once('=')).filter(|(name, _)| *name == key).last()?.1.trim().parse().ok()
}

fn ini_set(text: &str, key: &str, value: f32) -> String {
    let mut out: String = text.lines().filter(|line| line.split_once('=').map_or(true, |(name, _)| name != key)).map(|line| format!("{line}\n")).collect();
    out.push_str(&format!("{key}={value}\n"));
    out
}

#[derive(serde::Serialize)]
struct Mod {
    id: &'static str,
    name: &'static str,
    note: &'static str,
    on: bool,
}

#[derive(serde::Serialize)]
struct State {
    /// (name, whether Talaz downloaded it and may delete it)
    versions: Vec<(String, bool)>,
    email: Option<String>,
    mods: Vec<Mod>,
}

#[tauri::command]
fn state() -> State {
    let own = root().map(|root| root.join("versions"));
    let settings = profile_file().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    State {
        versions: installed().into_iter().map(|(name, path)| (name, own.as_deref().is_some_and(|own| path.starts_with(own)))).collect(),
        email: play::email(),
        mods: MODS.iter().map(|&(id, name, note)| Mod { id, name, note, on: ini_get(&settings, &format!("{id}.enabled")).is_some_and(|v| v != 0.0) }).collect(),
    }
}

/// Turns a mod on or off for the next start of the game
// ponytail: a running game writes its own settings back over this. Tell the mod instead once the two can talk
#[tauri::command]
fn set_mod(id: String, on: bool) -> Result<(), String> {
    if !MODS.iter().any(|(known, ..)| *known == id) {
        return Err("No such mod".into());
    }
    let file = profile_file().ok_or("No home folder")?;
    std::fs::create_dir_all(mods_dir().ok_or("No home folder")?).map_err(|err| err.to_string())?;
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(file, ini_set(&text, &format!("{id}.enabled"), on as u8 as f32)).map_err(|err| err.to_string())
}

const NOTES_URL: &str = "https://minecraft.wiki/api.php?action=query&prop=extracts&exintro=1&explaintext=1&exlimit=max&redirects=1&format=json";

/// The wiki page about a version: 1.21.114.1 is on "Bedrock Edition 1.21.114".
/// From 1.26 on the game goes by the year, so 1.26.60.4 is on "Bedrock Edition 26.60"
fn wiki_title(version: &str) -> String {
    let parts = version_key(version);
    let named: Vec<String> = match parts[..] {
        [1, year, drop, ..] if year >= 26 => vec![year.to_string(), drop.to_string()],
        _ => parts.iter().take(3).map(u32::to_string).collect(),
    };
    format!("Bedrock Edition {}", named.join("."))
}

/// The opening lines of each page in the wiki's answer, by the title that was asked for
fn notes_from(json: &str) -> HashMap<String, String> {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let query = &value["query"];
    let mut notes: HashMap<String, String> = query["pages"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, page)| Some((page["title"].as_str()?.to_string(), page["extract"].as_str().filter(|text| !text.is_empty())?.to_string())))
        .collect();
    for moved in query["redirects"].as_array().into_iter().flatten() {
        if let (Some(from), Some(note)) = (moved["from"].as_str(), moved["to"].as_str().and_then(|to| notes.get(to)).cloned()) {
            notes.insert(from.to_string(), note);
        }
    }
    notes
}

/// What each version changed, in a sentence or two from the Minecraft Wiki. Versions it has no page for are left out
#[tauri::command(async)]
fn notes(versions: Vec<String>) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    // the wiki answers 20 pages at a time
    for chunk in versions.chunks(20) {
        let titles = chunk.iter().map(|version| wiki_title(version)).collect::<Vec<_>>().join("|");
        let text = ureq::get(NOTES_URL).query("titles", &titles).header("user-agent", "Talaz launcher").call().map_err(|err| format!("Could not load the version notes: {err}"))?.body_mut().read_to_string().map_err(|err| err.to_string())?;
        let found = notes_from(&text);
        out.extend(chunk.iter().filter_map(|version| Some((version.clone(), found.get(&wiki_title(version))?.clone()))));
    }
    Ok(out)
}

/// Deletes a version Talaz downloaded. Versions from Minecraft Bedrock Launcher are left alone
#[tauri::command]
fn remove(version: String) -> Result<(), String> {
    let own = root().ok_or("No home folder")?.join("versions");
    let (_, path) = installed().into_iter().find(|(name, path)| *name == version && path.starts_with(&own)).ok_or("Talaz did not download that version")?;
    std::fs::remove_dir_all(path).map_err(|err| err.to_string())
}

/// Shows the folder with the worlds and resource packs
#[tauri::command]
fn open_game_folder() -> Result<(), String> {
    let dir = root().ok_or("No home folder")?.join("game");
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(dir).spawn().map(drop).map_err(|err| err.to_string())
}

#[tauri::command(async)]
fn play(app: tauri::AppHandle, version: String, width: u32, height: u32) -> Result<(), String> {
    // the name comes from the page, so only take one that is installed
    let (_, game) = installed().into_iter().find(|(name, _)| *name == version).ok_or("That version is not installed")?;
    runtime::ensure(&app)?;
    let client = runtime::tool("mcpelauncher-client").ok_or("The game runtime is missing")?;
    let mut cmd = Command::new(client);
    // without this the Xbox sign-in offers a passkey prompt that never opens
    cmd.env("QTWEBENGINE_CHROMIUM_FLAGS", "--disable-blink-features=WebAuth");
    cmd.arg("-dg").arg(game);
    if let Some(data) = root().map(|root| root.join("game")) {
        // first start: hide mcpelauncher's own menu bar over the game
        let settings = data.join("mcpelauncher-client-settings.txt");
        if !settings.exists() {
            std::fs::create_dir_all(&data).map_err(|err| err.to_string())?;
            std::fs::write(&settings, "enable_menubar=false\n").map_err(|err| err.to_string())?;
        }
        cmd.arg("-dd").arg(data).arg("-ww").arg(width.clamp(640, 7680).to_string()).arg("-wh").arg(height.clamp(360, 4320).to_string());
    }
    cmd.arg("-m").arg(place_mod()?);
    cmd.spawn().map(drop).map_err(|err| err.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![state, play, set_mod, remove, notes, open_game_folder, play::available, play::sign_in, play::sign_out, play::install])
        .run(tauri::generate_context!())
        .expect("failed to start Talaz");
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "writes into the real Talaz folder"]
    fn the_mod_is_placed() {
        let dir = super::place_mod().unwrap();
        assert_eq!(std::fs::read(dir.join("libtalaz_bedrock.so")).unwrap(), super::MOD);
    }

    #[test]
    fn a_setting_is_replaced_not_repeated() {
        let text = super::ini_set("zoom.enabled=0\nzoom.factor=4\n", "zoom.enabled", 1.0);
        assert_eq!(text, "zoom.factor=4\nzoom.enabled=1\n");
        assert_eq!(super::ini_get(&text, "zoom.enabled"), Some(1.0));
    }

    #[test]
    fn notes_follow_redirects_and_skip_missing_pages() {
        let json = r#"{"query":{"redirects":[{"from":"Bedrock Edition 1.2.3","to":"Bedrock Edition 1.2"}],"pages":{"-1":{"title":"Bedrock Edition 9.9.9","missing":""},"7":{"title":"Bedrock Edition 1.2","extract":"Adds bees."}}}}"#;
        let notes = super::notes_from(json);
        assert_eq!(notes[&super::wiki_title("1.2.3.4")], "Adds bees.");
        assert!(!notes.contains_key("Bedrock Edition 9.9.9"));
        assert!(super::notes_from("not json").is_empty());
        assert_eq!(super::wiki_title("1.26.60.4"), "Bedrock Edition 26.60");
    }

    #[test]
    #[ignore = "asks the Minecraft Wiki"]
    fn the_wiki_knows_the_recommended_version() {
        let notes = super::notes(vec!["1.21.114.1".into(), "1.26.60.4".into(), "0.0.0.0".into()]).unwrap();
        assert_eq!(notes.len(), 2);
        println!("{notes:?}");
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(super::version_key("1.26.60.4") > super::version_key("1.21.114.1"));
    }
}
