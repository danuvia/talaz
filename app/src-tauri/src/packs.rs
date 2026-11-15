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
    /// pack_icon.png as a data URL, for the list
    icon: Option<String>,
    size: u64,
    /// unix seconds the folder was made
    added: u64,
    /// the CurseForge project it came from, when Browse installed it
    curseforge: Option<u64>,
}

/// Marks a pack Browse installed, so it isn't offered twice
pub fn mark_source(folder: &str, id: u64) -> Result<(), String> {
    std::fs::write(mojang()?.join("resource_packs").join(folder).join("talaz-source.json"), format!(r#"{{"curseforge":{id}}}"#)).map_err(|e| e.to_string())
}

fn base64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map_or(0, |m| m.len()) }).sum()
}

impl Pack {
    pub fn curseforge(&self) -> Option<u64> {
        self.curseforge
    }
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
        let path = entry.path();
        // icons over 256 KB would bloat the page; those packs show a plain tile
        let icon = std::fs::read(path.join("pack_icon.png")).ok().filter(|b| b.len() < 256 * 1024).map(|b| format!("data:image/png;base64,{}", base64(&b)));
        let curseforge = std::fs::read_to_string(path.join("talaz-source.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["curseforge"].as_u64());
        let added = path.metadata().and_then(|m| m.created().or(m.modified())).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        list.push(Pack {
            icon,
            size: dir_size(&path),
            added,
            curseforge,
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

#[tauri::command]
pub fn remove_pack(folder: String) -> Result<(), String> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') || folder == "talaz" {
        return Err("Not a pack".into());
    }
    set_pack(folder.clone(), false).ok();
    std::fs::remove_dir_all(mojang()?.join("resource_packs").join(folder)).map_err(|e| e.to_string())
}
