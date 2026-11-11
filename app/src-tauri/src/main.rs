#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, process::Command};

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

// ponytail: the mod is loaded from the repo's build folder. Bundle it with the app for releases
fn mods_dir() -> Option<PathBuf> {
    let abi = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mod/build").join(abi);
    dir.exists().then_some(dir)
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

#[derive(serde::Serialize)]
struct State {
    versions: Vec<String>,
    email: Option<String>,
}

#[tauri::command]
fn state() -> State {
    State { versions: installed().into_iter().map(|(name, _)| name).collect(), email: play::email() }
}

#[tauri::command(async)]
fn play(app: tauri::AppHandle, version: String) -> Result<(), String> {
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
        cmd.arg("-dd").arg(data).args(["-ww", "1280", "-wh", "720"]);
    }
    if let Some(mods) = mods_dir() {
        cmd.arg("-m").arg(mods);
    }
    cmd.spawn().map(drop).map_err(|err| err.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![state, play, play::available, play::sign_in, play::sign_out, play::install])
        .run(tauri::generate_context!())
        .expect("failed to start Talaz");
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_by_number() {
        assert!(super::version_key("1.26.60.4") > super::version_key("1.21.114.1"));
    }
}
