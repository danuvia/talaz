#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, process::Command};

/// Where mcpelauncher keeps the game versions and the sign-in
fn data_root() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(home.join(if cfg!(target_os = "macos") {
        "Library/Application Support/mcpelauncher"
    } else {
        ".local/share/mcpelauncher"
    }))
}

// ponytail: finds the mac app or a binary on PATH, not the flatpak. Goes away once Talaz ships its own build
fn client() -> Option<PathBuf> {
    let mac = PathBuf::from("/Applications/Minecraft Bedrock Launcher.app/Contents/MacOS/mcpelauncher-client");
    if mac.exists() {
        return Some(mac);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("mcpelauncher-client")).find(|path| path.exists())
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

/// Installed game versions, newest first
#[tauri::command]
fn versions() -> Vec<String> {
    let Some(root) = data_root() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(root.join("versions")) else { return Vec::new() };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    found.sort_by_key(|version| std::cmp::Reverse(version_key(version)));
    found
}

#[tauri::command]
fn play(version: String) -> Result<(), String> {
    // the name comes from the page, so only take one that is installed
    if !versions().contains(&version) {
        return Err("That version is not installed".into());
    }
    let client = client().ok_or("mcpelauncher is not installed")?;
    let game = data_root().ok_or("No home folder")?.join("versions").join(&version);
    let mut cmd = Command::new(client);
    // without this the Xbox sign-in offers a passkey prompt that never opens
    cmd.env("QTWEBENGINE_CHROMIUM_FLAGS", "--disable-blink-features=WebAuth");
    cmd.arg("-dg").arg(game);
    if let Some(mods) = mods_dir() {
        cmd.arg("-m").arg(mods);
    }
    cmd.spawn().map(drop).map_err(|err| err.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![versions, play])
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
