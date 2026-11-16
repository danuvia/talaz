#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use tauri::Emitter;

/// Whether a game Talaz started is still running, so a second Play waits
static RUNNING: AtomicBool = AtomicBool::new(false);
/// the running game's process, for Stop
static GAME_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

mod device;
mod play;
mod curseforge;
mod instances;
mod packs;
mod runtime;
#[cfg(windows)]
mod win;

fn home() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?))
}

/// Everything Talaz keeps: the runtime, the game versions, the game's own data and the sign-in
pub fn root() -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Talaz"));
    }
    Some(home()?.join(if cfg!(target_os = "macos") { "Library/Application Support/Talaz" } else { ".local/share/talaz" }))
}

/// The file that says where the game keeps its worlds. It is written when the first-run setup ends
fn game_folder_file() -> Option<PathBuf> {
    Some(root()?.join("game-folder.txt"))
}

/// The game's own data: worlds, resource packs, settings. Picked in the setup, Talaz's folder until then
fn game_dir() -> Option<PathBuf> {
    // the Windows game keeps its data where Minecraft decides, nothing to pick
    // ponytail: the shared folder of the GDK builds, from what players report; per account folders sit beside it under Users
    if cfg!(windows) {
        return Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Minecraft Bedrock/Users/Shared"));
    }
    let instance = instances::selected();
    if !instance.shared {
        return instances::folder(&instance.id);
    }
    let picked = game_folder_file().and_then(|file| std::fs::read_to_string(file).ok()).map(|text| PathBuf::from(text.trim()));
    picked.filter(|dir| dir.is_absolute()).or_else(|| Some(root()?.join("game")))
}

/// Folders that hold game versions: Talaz's own, then mcpelauncher's so versions downloaded there still show up
fn version_dirs() -> Vec<PathBuf> {
    let theirs = home().map(|home| home.join(if cfg!(target_os = "macos") { "Library/Application Support/mcpelauncher" } else { ".local/share/mcpelauncher" }));
    root().into_iter().chain(theirs).map(|dir| dir.join("versions")).collect()
}

/// The mod, built before the launcher and carried inside it
#[cfg(windows)]
const MOD: &[u8] = include_bytes!("../../../mod/build/windows/libtalaz_bedrock.dll");
#[cfg(all(not(windows), target_arch = "aarch64"))]
const MOD: &[u8] = include_bytes!("../../../mod/build/arm64/libtalaz_bedrock.so");
#[cfg(all(not(windows), target_arch = "x86_64"))]
const MOD: &[u8] = include_bytes!("../../../mod/build/x86_64/libtalaz_bedrock.so");
const MOD_FILE: &str = if cfg!(windows) { "libtalaz_bedrock.dll" } else { "libtalaz_bedrock.so" };

/// Where the game loads the mod from. The mod keeps its settings next to itself
fn mods_dir() -> Option<PathBuf> {
    Some(root()?.join("mod"))
}

/// Writes the mod out when it is missing or from another build of Talaz
fn place_mod() -> Result<PathBuf, String> {
    let dir = mods_dir().ok_or("No home folder")?;
    let lib = dir.join(MOD_FILE);
    if std::fs::read(&lib).ok().as_deref() != Some(MOD) {
        std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        // a running game still has the old file open, so swap it instead of writing into it
        let part = dir.join(format!("{MOD_FILE}.part"));
        std::fs::write(&part, MOD).and_then(|_| std::fs::rename(&part, &lib)).map_err(|err| err.to_string())?;
    }
    Ok(dir)
}

fn version_key(version: &str) -> Vec<u32> {
    version.split('.').filter_map(|part| part.parse().ok()).collect()
}

/// What is known to go wrong with a version, if anything
// ponytail: mcpelauncher keeps no list of problems per version, so this is what we saw ourselves. Move to a file fetched from the repo once the list grows
#[tauri::command]
fn warning(version: String) -> Option<&'static str> {
    let key = version_key(&version);
    if cfg!(target_os = "macos") && key >= vec![1, 26] {
        return Some("Minecraft 1.26 does not start on a Mac yet. Pick 1.21.114.1 instead.");
    }
    (key > version_key("1.21.114.1")).then_some("This version is not tested with Talaz. It may not start, and mods may not work.")
}

/// Installed game versions and where they are, newest first
fn installed() -> Vec<(String, PathBuf)> {
    // Windows has the one version the Store installed
    #[cfg(windows)]
    return if win::installed() { vec![("Minecraft for Windows".into(), PathBuf::new())] } else { Vec::new() };
    #[allow(unreachable_code)]
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

/// Every mod and its settings, from the mod's own list; the Mods page is built from it
const MODS_DEF: &str = include_str!("../../../mod/src/mods.def");
/// The Lua mods' header lines, gathered by build.rs from mod/scripts
const SCRIPTS: &str = include_str!(concat!(env!("OUT_DIR"), "/scripts.txt"));

struct DefMod {
    id: &'static str,
    name: &'static str,
    note: &'static str,
    on: bool,
    /// a Lua mod from mod/scripts, or one built into the library
    lua: bool,
    /// whether it has settings, which are changed in the game
    settings: bool,
}

/// The arguments of a MOD(...) line, quotes taken off
fn def_args(line: &'static str) -> Vec<&'static str> {
    let inner = line.split_once('(').and_then(|(_, rest)| rest.rsplit_once(')')).map_or("", |(inner, _)| inner);
    let (mut out, mut start, mut quoted) = (Vec::new(), 0, false);
    for (i, c) in inner.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.push(inner[start..i].trim().trim_matches('"'));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(inner[start..].trim().trim_matches('"'));
    out
}

/// Every mod: the built-in ones from mods.def, then the Lua mods from their header lines
fn mods_def() -> &'static Vec<DefMod> {
    static DEF: std::sync::OnceLock<Vec<DefMod>> = std::sync::OnceLock::new();
    DEF.get_or_init(|| {
        let mut mods: Vec<DefMod> = Vec::new();
        for line in MODS_DEF.lines().map(str::trim) {
            let a = def_args(line);
            // GAMEMOD: needs hooks in the game, which Windows has none of yet
            if (line.starts_with("MOD(") || (line.starts_with("GAMEMOD(") && !cfg!(windows))) && a.len() == 4 {
                mods.push(DefMod { id: a[0], name: a[1], note: a[2], on: a[3] != "0", lua: false, settings: false });
            } else if line.starts_with("SLIDER(") || line.starts_with("SWITCH(") {
                if let Some(m) = mods.iter_mut().find(|m| a.first() == Some(&m.id)) {
                    m.settings = true;
                }
            }
        }
        // "--@ id zoom", "--@ slider ...", and so on. One that needs a hook leaves Windows
        // out, which has none yet
        let mut current: Option<DefMod> = None;
        let mut needs = false;
        let finish = |m: Option<DefMod>, needs: bool, mods: &mut Vec<DefMod>| {
            if let Some(m) = m.filter(|_| !(needs && cfg!(windows))) {
                mods.push(m);
            }
        };
        for line in SCRIPTS.lines() {
            if line.starts_with("--@@ ") {
                finish(current.take(), needs, &mut mods);
                needs = false;
                continue;
            }
            let Some((word, rest)) = line.strip_prefix("--@ ").map(|l| l.split_once(' ').unwrap_or((l, ""))) else { continue };
            match word {
                "id" => current = Some(DefMod { id: rest, name: rest, note: "", on: false, lua: true, settings: false }),
                "name" => if let Some(m) = &mut current { m.name = rest },
                "note" => if let Some(m) = &mut current { m.note = rest },
                "on" => if let Some(m) = &mut current { m.on = rest == "1" },
                "needs" => needs = true,
                "slider" | "switch" => if let Some(m) = &mut current { m.settings = true },
                _ => {}
            }
        }
        finish(current.take(), needs, &mut mods);
        mods
    })
}

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
    lua: bool,
    settings: bool,
}

#[derive(serde::Serialize)]
struct State {
    /// (name, whether Talaz downloaded it and may delete it)
    versions: Vec<(String, bool)>,
    email: Option<String>,
    mods: Vec<Mod>,
    running: bool,
    /// false until the first-run setup is done
    set_up: bool,
    game_folder: String,
    /// The Windows launcher has no Google sign-in, game downloads or game folder
    windows: bool,
    /// versions every mod works on here; on the others the ones hooked into the game stay off
    full_mods: Vec<&'static str>,
}

#[tauri::command]
fn state() -> State {
    let own = root().map(|root| root.join("versions"));
    let settings = profile_file().and_then(|file| std::fs::read_to_string(file).ok()).unwrap_or_default();
    State {
        versions: installed().into_iter().map(|(name, path)| (name, own.as_deref().is_some_and(|own| path.starts_with(own)))).collect(),
        email: play::email(),
        running: RUNNING.load(Ordering::SeqCst),
        set_up: game_folder_file().is_some_and(|file| file.exists()),
        game_folder: game_dir().map(|dir| dir.display().to_string()).unwrap_or_default(),
        windows: cfg!(windows),
        // ponytail: the game hooks are written for 1.21.114.1 on arm64 only; add versions and x86_64 here as they get their own
        full_mods: if cfg!(all(target_os = "macos", target_arch = "aarch64")) { vec!["1.21.114.1"] } else { vec![] },
        mods: mods_def()
            .iter()
            .map(|m| Mod {
                id: m.id,
                name: m.name,
                note: m.note,
                on: ini_get(&settings, &format!("{}.enabled", m.id)).map_or(m.on, |v| v != 0.0),
                lua: m.lua,
                settings: m.settings,
            })
            .collect(),
    }
}

/// Turns a mod on or off for the next start of the game
// ponytail: a running game writes its own settings back over this. Tell the mod instead once the two can talk
#[tauri::command]
fn set_mod(id: String, on: bool) -> Result<(), String> {
    if !mods_def().iter().any(|m| m.id == id) {
        return Err("No such mod".into());
    }
    save(&format!("{id}.enabled"), on as u8 as f32)
}

fn save(key: &str, value: f32) -> Result<(), String> {
    let file = profile_file().ok_or("No home folder")?;
    std::fs::create_dir_all(mods_dir().ok_or("No home folder")?).map_err(|err| err.to_string())?;
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(file, ini_set(&text, key, value)).map_err(|err| err.to_string())
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

/// The game's own graphics settings that cost the most frames
const FASTER: &[(&str, &str)] = &[
    ("gfx_vsync", "0"),
    ("gfx_fancyskies", "0"),
    ("gfx_smoothlighting", "0"),
    ("gfx_transparentleaves", "0"),
    ("gfx_viewdistance", "160"),
    ("gfx_toggleclouds", "0"),
];

/// The game's options.txt with some values changed; a setting the game has not written yet is left alone
fn options_set(text: &str, values: &[(&str, &str)]) -> String {
    text.lines()
        .map(|line| match line.split_once(':').and_then(|(key, _)| values.iter().find(|(known, _)| *known == key)) {
            Some((key, value)) => format!("{key}:{value}\n"),
            None => format!("{line}\n"),
        })
        .collect()
}

/// Turns the game's costly graphics settings down. The settings from before stay beside the file
// ponytail: a running game writes its own settings back on exit, so this only holds with the game closed
#[tauri::command]
fn faster_graphics() -> Result<(), String> {
    let file = game_dir().ok_or("No home folder")?.join("games/com.mojang/minecraftpe/options.txt");
    let text = std::fs::read_to_string(&file).map_err(|_| "Start the game once first, so it writes its settings")?;
    let backup = file.with_extension("txt.before-talaz");
    if !backup.exists() {
        std::fs::write(backup, &text).map_err(|err| err.to_string())?;
    }
    std::fs::write(file, options_set(&text, FASTER)).map_err(|err| err.to_string())
}

// The resource pack that lays out the mod's title screen. Its start screen is made
// from the game's own at every start, so the game's screen stays one click away and
// none of Mojang's files ship with Talaz
const PACK: &[(&str, &[u8])] = &[
    ("manifest.json", include_bytes!("../../../mod/pack/manifest.json")),
    ("textures/ui/talaz/clear.png", include_bytes!("../../../mod/pack/textures/ui/talaz/clear.png")),
    ("textures/ui/talaz/white.png", include_bytes!("../../../mod/pack/textures/ui/talaz/white.png")),
    ("textures/ui/talaz/saturation_full.png", include_bytes!("../../../mod/pack/textures/ui/talaz/saturation_full.png")),
    ("textures/ui/talaz/saturation_half.png", include_bytes!("../../../mod/pack/textures/ui/talaz/saturation_half.png")),
    ("textures/ui/talaz/exhaustion_1.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_1.png")),
    ("textures/ui/talaz/exhaustion_2.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_2.png")),
    ("textures/ui/talaz/exhaustion_3.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_3.png")),
    ("textures/ui/talaz/exhaustion_4.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_4.png")),
    ("textures/ui/talaz/exhaustion_5.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_5.png")),
    ("textures/ui/talaz/exhaustion_6.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_6.png")),
    ("textures/ui/talaz/exhaustion_7.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_7.png")),
    ("textures/ui/talaz/exhaustion_8.png", include_bytes!("../../../mod/pack/textures/ui/talaz/exhaustion_8.png")),
];

// name, the game's button, x, y, w, h as fractions of the screen; the same spots as mod/src/title.cpp
const TITLE_BUTTONS: &[(&str, &str, f64, f64, f64, f64)] = &[
    ("play", "button.menu_play", 0.07, 0.47, 0.27, 0.08),
    ("servers", "button.menu_servers", 0.07, 0.57, 0.27, 0.065),
    ("realms", "button.menu_realms", 0.07, 0.65, 0.27, 0.065),
    ("settings", "button.menu_settings", 0.07, 0.73, 0.13, 0.065),
    ("profile", "button.menu_profile", 0.74, 0.88, 0.115, 0.065),
    ("wardrobe", "button.to_profile_screen", 0.865, 0.88, 0.115, 0.065),
    ("inbox", "button.menu_inbox", 0.905, 0.03, 0.035, 0.06),
    ("quit", "button.menu_exit", 0.945, 0.03, 0.035, 0.06),
];

/// The game's UI files allow comments and trailing commas, which JSON doesn't
fn plain_json(text: &str) -> String {
    let (mut out, mut chars) = (String::with_capacity(text.len()), text.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    if c == '\\' {
                        out.extend(chars.next());
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                while chars.next_if(|&c| c != '\n').is_some() {}
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            ']' | '}' => {
                // drop a comma left before the closing bracket
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Talaz's start screen, with the game's own behind a switch in the corner
fn start_screen(vanilla: &str) -> Option<String> {
    use serde_json::{json, Value};
    let parsed: Value = serde_json::from_str(&plain_json(vanilla)).ok()?;
    let mut theirs = parsed.get("start_screen_content")?.get("controls")?.as_array()?.clone();
    let pct = |v: f64| format!("{:.2}%", v * 100.0);
    let place = |x: f64, y: f64, w: f64, h: f64| json!({"anchor_from": "top_left", "anchor_to": "top_left", "size": [pct(w), pct(h)], "offset": [pct(x), pct(y)]});
    // 2px marks the mod looks for, one colour on each screen, under buttons the mod paints solid:
    // Talaz's inside Play, where a dialog over the middle of the screen also hides it, the game's
    // inside the way back
    let mark = |name: &str, rgb: [f64; 3], x: &str, y: &str| {
        json!({name: {"type": "image", "texture": "textures/ui/talaz/white", "color": rgb.map(|c| c / 255.0), "size": [2, 2],
            "anchor_from": "top_left", "anchor_to": "top_left", "offset": [x, y], "layer": 100}})
    };
    let clear = "textures/ui/talaz/clear";
    let mut ours = vec![mark("talaz_mark", [1.0, 254.0, 3.0], "20%", "51%")];
    for &(name, target, x, y, w, h) in TITLE_BUTTONS {
        let mut button = place(x, y, w, h);
        // above the screen switch, which lies under the whole top row
        let fields = json!({"layer": 60, "$pressed_button_name": target, "$button_text": "", "$default_button_texture": clear, "$hover_button_texture": clear,
            "$pressed_button_texture": clear, "$locked_button_texture": clear, "$border_visible": false});
        button.as_object_mut()?.extend(fields.as_object()?.clone());
        ours.push(json!({format!("talaz_{name}@common_buttons.light_text_button"): button}));
    }
    theirs.push(mark("talaz_vanilla_mark", [3.0, 1.0, 254.0], "96%", "6%"));
    // the screen switch below lies under the whole top row; these take the clicks it shouldn't get,
    // a silent button pressing nothing, over the gaps beside the cube and left of the way back
    let block = |name: &str, x: f64, w: f64| {
        let mut button = place(x, 0.03, w, 0.06);
        let fields = json!({"layer": 55, "sound_volume": 0.0, "$pressed_button_name": "button.talaz_nothing", "$button_text": "", "$default_button_texture": clear,
            "$hover_button_texture": clear, "$pressed_button_texture": clear, "$locked_button_texture": clear, "$border_visible": false});
        button.as_object_mut().map(|o| o.extend(fields.as_object().cloned().unwrap_or_default()));
        json!({format!("{name}@common_buttons.light_text_button"): button})
    };
    ours.push(block("talaz_block_ours", 0.9, 0.08));
    theirs.push(block("talaz_block_theirs", 0.865, 0.08));
    // one toggle outside both screens switches them. It lies under the whole top row: on Talaz's
    // screen the inbox and quit buttons sit above it and its left end, the cube, is what's left;
    // the game's screen has nothing there, so the way back can sit at the right end
    let mut toggle = place(0.865, 0.03, 0.115, 0.06);
    let fields = json!({"type": "toggle", "layer": 50, "toggle_name": "talaz_screen", "toggle_default_state": false,
        "checked_control": "on", "unchecked_control": "off", "checked_hover_control": "on", "unchecked_hover_control": "off",
        "controls": [{"on": {"type": "panel"}}, {"off": {"type": "panel"}}],
        "button_mappings": [{"from_button_id": "button.menu_select", "to_button_id": "button.menu_select", "mapping_type": "pressed"},
            {"from_button_id": "button.menu_ok", "to_button_id": "button.menu_ok", "mapping_type": "focused"}]});
    toggle.as_object_mut()?.extend(fields.as_object()?.clone());
    let shown_when = |on: bool| json!([{"binding_type": "view", "source_control_name": "talaz_to_vanilla",
        "source_property_name": if on { "#toggle_state" } else { "(not #toggle_state)" }, "target_property_name": "#visible"}]);
    let ui = json!({"namespace": "start", "start_screen_content": {"controls": [
        {"talaz_screen": {"type": "panel", "bindings": shown_when(false), "controls": ours}},
        {"vanilla_screen": {"type": "panel", "bindings": shown_when(true), "controls": theirs}},
        {"talaz_to_vanilla": toggle},
    ]}});
    serde_json::to_string(&ui).ok()
}

/// Reads a PNG as 8-bit RGBA
fn rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let pixels = &buf[..info.buffer_size()];
    let data = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::GrayscaleAlpha => pixels.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, data))
}

/// The icons Food writes into item tooltips, as a glyph page the game's text can show:
/// a full and a half shank, the same for food that hungers you, then a full and a half
/// saturation mark. The shanks come from the game's own textures, so none of Mojang's
/// art ships with Talaz. 16-pixel cells with the 9-pixel icon three rows down, like the
/// game's own glyphs
fn food_glyphs(version: &std::path::Path) -> Option<Vec<u8>> {
    let game = |name: &str| {
        ["assets/assets/resource_packs/vanilla/textures/ui/", "assets/resource_packs/vanilla/textures/ui/", "data/resource_packs/vanilla/textures/ui/"]
            .iter()
            .find_map(|dir| std::fs::read(version.join(format!("{dir}{name}.png"))).ok())
    };
    let ours = |name: &str| PACK.iter().find(|(path, _)| path.ends_with(name)).map(|(_, bytes)| bytes.to_vec());
    let icons = [game("hunger_full"), game("hunger_half"), game("hunger_effect_full"), game("hunger_effect_half"),
                 ours("saturation_full.png"), ours("saturation_half.png")];
    let mut page = vec![0u8; 256 * 256 * 4];
    for (cell, icon) in icons.iter().enumerate() {
        let (w, h, data) = rgba(icon.as_ref()?)?;
        for y in 0..h.min(13) {
            for x in 0..w.min(16) {
                let from = ((y * w + x) * 4) as usize;
                let to = (((y + 3) * 256 + cell as u32 * 16 + x) * 4) as usize;
                page[to..to + 4].copy_from_slice(&data[from..from + 4]);
            }
        }
        // the game trims a glyph to its drawn columns, so a half icon, empty on the left,
        // gets a barely-there pixel there to keep a whole icon's width
        let corner = ((3 * 256 + cell as u32 * 16) * 4) as usize;
        if page[corner + 3] == 0 {
            page[corner + 3] = 1;
        }
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, 256, 256);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&page).ok()?;
    Some(out)
}

/// Puts the pack in the game's folder and turns it on for everything, first in the list
fn install_pack(game: &std::path::Path, version: &std::path::Path) -> std::io::Result<()> {
    let mojang = game.join("games/com.mojang");
    let dir = mojang.join("resource_packs/talaz");
    // the Android game keeps the vanilla pack in its assets, the Windows one under data
    let screen = ["assets/assets/resource_packs/vanilla/ui/start_screen.json", "assets/resource_packs/vanilla/ui/start_screen.json", "data/resource_packs/vanilla/ui/start_screen.json"]
        .iter()
        .find_map(|path| std::fs::read_to_string(version.join(path)).ok())
        .and_then(|vanilla| start_screen(&vanilla));
    let Some(screen) = screen else {
        // without a start screen of our own the game keeps its own; a stale one could miss new buttons
        let _ = std::fs::remove_file(dir.join("ui/start_screen.json"));
        return Ok(());
    };
    let glyphs = food_glyphs(version).map(|page| ("font/glyph_E2.png", page));
    let files = PACK.iter().map(|&(name, bytes)| (name, bytes.to_vec())).chain([("ui/start_screen.json", screen.into_bytes())]).chain(glyphs);
    for (name, bytes) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap_or(&dir))?;
        if std::fs::read(&path).ok() != Some(bytes.clone()) {
            std::fs::write(&path, bytes)?;
        }
    }
    let manifest = String::from_utf8_lossy(PACK[0].1);
    let uuid = manifest.split("\"uuid\"").nth(1).and_then(|rest| rest.split('"').nth(1)).unwrap_or_default();
    let list = mojang.join("minecraftpe/global_resource_packs.json");
    let current = std::fs::read_to_string(&list).unwrap_or_default();
    if uuid.is_empty() || current.contains(uuid) {
        return Ok(());
    }
    let entry = format!(r#"{{"pack_id":"{uuid}","version":[1,0,0]}}"#);
    let updated = match current.trim().strip_prefix('[') {
        Some(rest) if rest.trim_start().starts_with(']') => format!("[{entry}]"),
        Some(rest) => format!("[{entry},{rest}"),
        None => format!("[{entry}]"),
    };
    std::fs::create_dir_all(list.parent().unwrap_or(&mojang))?;
    std::fs::write(list, updated)
}

/// Asks for a folder with the system's own picker. Nothing when the user cancels
#[tauri::command(async)]
fn pick_folder() -> Result<Option<String>, String> {
    let out = if cfg!(target_os = "macos") {
        Command::new("osascript").args(["-e", "POSIX path of (choose folder with prompt \"Where should the game keep its worlds?\")"]).output()
    } else {
        runtime::system("zenity").args(["--file-selection", "--directory"]).output()
    }
    .map_err(|err| err.to_string())?;
    let path = String::from_utf8_lossy(&out.stdout).trim().trim_end_matches('/').to_string();
    Ok((out.status.success() && !path.is_empty()).then_some(path))
}

/// Ends the first-run setup with the folder the game keeps its worlds in
#[tauri::command]
fn finish_setup(folder: String) -> Result<(), String> {
    let dir = PathBuf::from(&folder);
    if !dir.is_absolute() {
        return Err("Pick a folder".into());
    }
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let file = game_folder_file().ok_or("No home folder")?;
    std::fs::create_dir_all(file.parent().ok_or("No home folder")?).map_err(|err| err.to_string())?;
    std::fs::write(file, folder).map_err(|err| err.to_string())
}

/// Minecraft's page in the Microsoft Store, for a Windows player who hasn't installed it
#[tauri::command]
fn open_store() -> Result<(), String> {
    Command::new("cmd").args(["/c", "start", "", "ms-windows-store://pdp/?productid=9NBLGGH2JHXJ"]).spawn().map(drop).map_err(|err| err.to_string())
}

/// Stops the running game, like closing its window
#[tauri::command]
fn stop_game() -> Result<(), String> {
    let pid = GAME_PID.load(Ordering::SeqCst);
    if pid == 0 || !RUNNING.load(Ordering::SeqCst) {
        return Err("The game isn't running".into());
    }
    let mut kill = if cfg!(windows) { let mut c = Command::new("taskkill"); c.args(["/PID", &pid.to_string(), "/F"]); c } else { let mut c = Command::new("kill"); c.arg(pid.to_string()); c };
    kill.status().map(drop).map_err(|e| e.to_string())
}

/// Shows the folder with the worlds and resource packs
#[tauri::command]
fn open_game_folder() -> Result<(), String> {
    let dir = game_dir().ok_or("No home folder")?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    runtime::system(if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(dir).spawn().map(drop).map_err(|err| err.to_string())
}

#[tauri::command(async)]
fn play(app: tauri::AppHandle, version: String, width: u32, height: u32) -> Result<(), String> {
    #[cfg(windows)]
    return win::play(app, place_mod()?).map(|_| drop((version, width, height)));
    #[allow(unreachable_code)]
    // the name comes from the page, so only take one that is installed
    let (_, game) = installed().into_iter().find(|(name, _)| *name == version).ok_or("That version is not installed")?;
    runtime::ensure(&app)?;
    // ponytail: a vanilla instance only skips the mod on Mac and Linux; Windows still injects it
    let vanilla = instances::selected().vanilla;
    let mods = place_mod()?;
    // the shader packs that are on, for the mod's material loader; none for a vanilla game
    let _ = std::fs::write(mods.join("shaders.txt"), if vanilla { String::new() } else { packs::shader_folders().join("\n") });
    let data = game_dir();
    let reach: Vec<&std::path::Path> = [Some(game.as_path()), data.as_deref(), Some(mods.as_path())].into_iter().flatten().collect();
    let mut cmd = runtime::command("mcpelauncher-client", &reach).ok_or("The game runtime is missing")?;
    // without this the Xbox sign-in offers a passkey prompt that never opens
    cmd.env("QTWEBENGINE_CHROMIUM_FLAGS", "--disable-blink-features=WebAuth");
    // on Linux the graphics driver holds vsync on whatever the game asks, so when the game's
    // own setting has it off the drivers are told too: Mesa's and NVIDIA's switches
    let vsync_off = data.as_ref().and_then(|dir| std::fs::read_to_string(dir.join("games/com.mojang/minecraftpe/options.txt")).ok())
        .is_some_and(|text| text.lines().any(|line| line.trim() == "gfx_vsync:0"));
    if cfg!(target_os = "linux") && vsync_off {
        cmd.env("vblank_mode", "0").env("__GL_SYNC_TO_VBLANK", "0");
    }
    cmd.arg("-dg").arg(&game);
    if let Some(data) = data {
        // first start: hide mcpelauncher's own menu bar over the game
        let settings = data.join("mcpelauncher-client-settings.txt");
        if !settings.exists() {
            std::fs::create_dir_all(&data).map_err(|err| err.to_string())?;
            std::fs::write(&settings, "enable_menubar=false\n").map_err(|err| err.to_string())?;
        }
        // without the pack the game shows its own title screen, so a failure here is not worth stopping for
        if !vanilla {
            let _ = install_pack(&data, &game);
        }
        cmd.arg("-dd").arg(data).arg("-ww").arg(width.clamp(640, 7680).to_string()).arg("-wh").arg(height.clamp(360, 4320).to_string());
    }
    if !vanilla {
        cmd.arg("-m").arg(mods);
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Minecraft is already running".into());
    }
    // The game writes straight into logs/latest.log rather than through the launcher, so
    // closing the launcher leaves it nothing broken to write to. The last game's log is kept
    // as previous.log, so a crash is still there after starting again
    // each instance keeps its own logs, in its folder; the last game's log is kept under its time
    let log = instances::logs_dir().map(|dir| dir.join("latest.log"));
    let file = log.as_ref().and_then(|path| {
        std::fs::create_dir_all(path.parent()?).ok()?;
        instances::rotate(path.parent()?);
        std::fs::File::create(path).ok()
    });
    let (out, err) = match file.as_ref().map(|file| (file.try_clone(), file.try_clone())) {
        Some((Ok(out), Ok(err))) => (Stdio::from(out), Stdio::from(err)),
        _ => (Stdio::null(), Stdio::null()),
    };
    let mut child = match cmd.stdout(out).stderr(err).spawn() {
        Ok(child) => child,
        Err(err) => {
            RUNNING.store(false, Ordering::SeqCst);
            return Err(err.to_string());
        }
    };
    GAME_PID.store(child.id(), Ordering::SeqCst);
    let _ = app.emit("game", true);
    let (instance, started) = (instances::selected().id, instances::start_time());
    // the page follows the file while the game runs, in batches since the game can print
    // thousands of lines a second
    if let Some(path) = log {
        let log_app = app.clone();
        std::thread::spawn(move || {
            let Ok(file) = std::fs::File::open(path) else { return };
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            while RUNNING.load(Ordering::SeqCst) {
                let mut lines = Vec::new();
                // a line the game is still writing waits for its end
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line.ends_with('\n') {
                    lines.push(line.trim_end().to_string());
                    line.clear();
                }
                if !lines.is_empty() {
                    let _ = log_app.emit("log", lines);
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        });
    }
    std::thread::spawn(move || {
        let _ = child.wait();
        instances::record(&instance, started);
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("game", false);
    });
    Ok(())
}

/// Looks for a newer Talaz on GitHub Releases at start, installs it and starts again,
/// like OneClient. The progress shows in the page's usual toast
fn update(app: tauri::AppHandle) {
    // a build run from the source tree updates through git, not here
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_updater::UpdaterExt;
        let Ok(updater) = app.updater() else { return };
        let Ok(Some(update)) = updater.check().await else { return };
        let text = format!("Updating Talaz to {}", update.version);
        let mut done = 0;
        let installed = update
            .download_and_install(
                |chunk, total| {
                    done += chunk;
                    runtime::progress(&app, &text, total.map(|total| (done as f64 / total as f64).min(1.0)));
                },
                || {},
            )
            .await;
        match installed {
            // a running game keeps going; the new Talaz opens next time
            Ok(()) if !RUNNING.load(Ordering::SeqCst) => app.restart(),
            Ok(()) => runtime::progress(&app, "Talaz updates when it next starts", Some(1.0)),
            Err(err) => runtime::progress(&app, &format!("Couldn't update Talaz: {err}"), None),
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            update(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, play, set_mod, remove, notes, warning, faster_graphics, open_game_folder, open_store, pick_folder, finish_setup, play::available, play::sign_in, play::sign_out, play::install, packs::packs, packs::set_pack, packs::import_pack, packs::remove_pack, curseforge::browse, curseforge::install_pack, curseforge::open_page, curseforge::pack_details, instances::instances, instances::select_instance, instances::create_instance, instances::update_instance, instances::delete_instance, instances::worlds, instances::delete_world, instances::open_world, instances::logs, instances::read_log, instances::delete_log, stop_game])
        .run(tauri::generate_context!())
        .expect("failed to start Talaz");
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs 1.21.114.1 installed through mcpelauncher"]
    fn the_installed_start_screen_turns_into_ours() {
        let file = super::home().unwrap().join("Library/Application Support/mcpelauncher/versions/1.21.114.1/assets/assets/resource_packs/vanilla/ui/start_screen.json");
        assert!(super::start_screen(&std::fs::read_to_string(file).unwrap()).is_some());
    }

    #[test]
    #[ignore = "writes the pack into the real game folder"]
    fn install_the_pack_for_a_look() {
        let version = super::home().unwrap().join("Library/Application Support/mcpelauncher/versions/1.21.114.1");
        super::install_pack(&super::game_dir().unwrap(), &version).unwrap();
    }

    #[test]
    fn the_games_ui_files_read_as_json() {
        let text = "{ // a comment\n \"a\": [1, 2,], /* more */ \"b\": \"// not a comment\", }";
        let value: serde_json::Value = serde_json::from_str(&super::plain_json(text)).unwrap();
        assert_eq!(value["a"][1], 2);
        assert_eq!(value["b"], "// not a comment");
        let screen = super::start_screen("{\"start_screen_content\": {\"controls\": [{\"x\": {}},]}}").unwrap();
        assert!(screen.contains("talaz_play") && screen.contains("vanilla_screen"));
    }

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
    fn the_mods_come_from_the_mods_own_list() {
        let mods = super::mods_def();
        assert!(mods.iter().any(|m| m.id == "riptide" && m.on && m.name == "Riptide" && !m.lua));
        assert!(mods.iter().any(|m| m.id == "zoom" && !m.on && m.lua && m.settings));
        assert!(mods.iter().any(|m| m.id == "fps" && m.lua && !m.settings));
    }

    #[test]
    fn game_options_change_only_where_asked() {
        let text = super::options_set("gfx_vsync:1\ngfx_gamma:0.5\n", &[("gfx_vsync", "0"), ("gfx_new", "1")]);
        assert_eq!(text, "gfx_vsync:0\ngfx_gamma:0.5\n");
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
        assert!(super::warning("1.21.114.1".into()).is_none() && super::warning("1.21.50.7".into()).is_none());
        assert!(super::warning("1.21.130.3".into()).is_some() && super::warning("1.26.60.4".into()).is_some());
    }
}
