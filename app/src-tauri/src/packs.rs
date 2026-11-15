//! The Resource Packs page: the packs in the game's folder, which ones the game starts
//! with (global_resource_packs.json, first is on top), importing a .mcpack and removing one

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct Pack {
    folder: String,
    name: String,
    description: String,
    uuid: String,
    on: bool,
}

fn mojang() -> Result<PathBuf, String> {
    Ok(super::game_dir().ok_or("No home folder")?.join("games/com.mojang"))
}

fn global_list(mojang: &Path) -> Vec<Value> {
    std::fs::read_to_string(mojang.join("minecraftpe/global_resource_packs.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<Value>>(&text).ok())
        .unwrap_or_default()
}

fn header(dir: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.get("header").cloned()
}

/// Minecraft color codes (§a) don't mean anything here
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[tauri::command]
pub fn packs() -> Result<Vec<Pack>, String> {
    let mojang = mojang()?;
    let on: Vec<String> = global_list(&mojang).iter().filter_map(|e| e["pack_id"].as_str().map(str::to_string)).collect();
    let mut list = Vec::new();
    for entry in std::fs::read_dir(mojang.join("resource_packs")).into_iter().flatten().flatten() {
        let folder = entry.file_name().to_string_lossy().to_string();
        // Talaz's own pack is the mods' and stays on
        if folder == "talaz" {
            continue;
        }
        let Some(h) = header(&entry.path()) else { continue };
        let uuid = h["uuid"].as_str().unwrap_or_default().to_string();
        list.push(Pack {
            name: plain(h["name"].as_str().unwrap_or(&folder)),
            description: plain(h["description"].as_str().unwrap_or_default()),
            on: on.contains(&uuid),
            uuid,
            folder,
        });
    }
    list.sort_by(|a, b| b.on.cmp(&a.on).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(list)
}

/// Turns a pack on, under Talaz's own so the mods' textures still win, or off
#[tauri::command]
pub fn set_pack(folder: String, on: bool) -> Result<(), String> {
    let mojang = mojang()?;
    let h = header(&mojang.join("resource_packs").join(&folder)).ok_or("That pack is gone")?;
    let uuid = h["uuid"].as_str().ok_or("The pack has no id")?;
    let version = h.get("version").cloned().unwrap_or(json!([1, 0, 0]));
    let mut list: Vec<Value> = global_list(&mojang).into_iter().filter(|e| e["pack_id"] != uuid).collect();
    if on {
        list.push(json!({"pack_id": uuid, "version": version}));
    }
    let file = mojang.join("minecraftpe/global_resource_packs.json");
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(file, serde_json::to_string(&list).unwrap()).map_err(|e| e.to_string())
}

/// Unpacks a .mcpack or .zip into the resource packs, in a folder of its own. A pack zipped
/// with its folder inside is fine too; the folder with the manifest is the one kept
#[tauri::command(async)]
pub fn import_pack(path: String) -> Result<String, String> {
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "That isn't a resource pack file")?;
    let packs = mojang()?.join("resource_packs");
    let stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("pack".into());
    let safe: String = stem.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let mut dest = packs.join(&safe);
    for i in 2.. {
        if !dest.exists() {
            break;
        }
        dest = packs.join(format!("{safe} {i}"));
    }
    let staging = packs.join(format!(".import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    // enclosed_name refuses paths that would land outside the folder
    zip.extract(&staging).map_err(|e| e.to_string())?;
    let root = if staging.join("manifest.json").exists() {
        Some(staging.clone())
    } else {
        std::fs::read_dir(&staging).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.join("manifest.json").exists())
    };
    let result = match root.as_ref().and_then(|r| header(r).map(|_| r)) {
        Some(root) => std::fs::rename(root, &dest).map_err(|e| e.to_string()),
        None => Err("No manifest.json in it, so it isn't a resource pack".to_string()),
    };
    let _ = std::fs::remove_dir_all(&staging);
    result?;
    let folder = dest.file_name().unwrap().to_string_lossy().to_string();
    set_pack(folder.clone(), true)?;
    Ok(folder)
}

/// Asks for a .mcpack with the system's picker
#[tauri::command(async)]
pub fn pick_pack() -> Result<Option<String>, String> {
    use std::process::Command;
    let out = if cfg!(target_os = "macos") {
        Command::new("osascript").args(["-e", "POSIX path of (choose file with prompt \"Pick a resource pack (.mcpack or .zip)\")"]).output()
    } else if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = 'Resource packs|*.mcpack;*.zip'; if ($d.ShowDialog() -eq 'OK') { $d.FileName }"])
            .output()
    } else {
        super::runtime::system("zenity").args(["--file-selection", "--file-filter=*.mcpack *.zip"]).output()
    }
    .map_err(|e| e.to_string())?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((out.status.success() && !path.is_empty()).then_some(path))
}

#[tauri::command]
pub fn remove_pack(folder: String) -> Result<(), String> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') || folder == "talaz" {
        return Err("Not a pack".into());
    }
    set_pack(folder.clone(), false).ok();
    std::fs::remove_dir_all(mojang()?.join("resource_packs").join(folder)).map_err(|e| e.to_string())
}
