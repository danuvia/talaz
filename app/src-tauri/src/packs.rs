//! The Packs page: the resource packs in the game's folder, which ones the game starts with
//! (global_resource_packs.json, first is on top), and the add-ons' behavior packs, which a
//! world turns on in its own settings. Importing a .mcpack or .mcaddon, and removing one

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
    /// "resource", or "behavior" for an add-on's part a world turns on
    kind: &'static str,
    /// a resource pack with its own renderer materials: a shader pack
    shader: bool,
}

/// The material folders of the shader packs that are on, top pack first, for the mod to
/// answer the game's materials from
pub fn shader_folders() -> Vec<String> {
    let Ok(mojang) = mojang() else { return Vec::new() };
    let packs: Vec<(String, PathBuf)> = std::fs::read_dir(mojang.join("resource_packs"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((header(&e.path())?["uuid"].as_str()?.to_string(), e.path())))
        .collect();
    global_list(&mojang)
        .iter()
        .filter_map(|entry| packs.iter().find(|(uuid, _)| entry["pack_id"] == uuid.as_str()))
        .map(|(_, path)| path.join("renderer/materials"))
        .filter(|dir| dir.is_dir())
        .map(|dir| dir.to_string_lossy().to_string())
        .collect()
}

/// The game's folder for each kind of pack
fn kind_dir(kind: &str) -> &'static str {
    if kind == "behavior" { "behavior_packs" } else { "resource_packs" }
}

/// Marks a pack Browse installed, so it isn't offered twice
pub fn mark_source(kind: &str, folder: &str, id: u64) -> Result<(), String> {
    std::fs::write(mojang()?.join(kind_dir(kind)).join(folder).join("talaz-source.json"), format!(r#"{{"curseforge":{id}}}"#)).map_err(|e| e.to_string())
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
    for kind in ["resource", "behavior"] {
        for entry in std::fs::read_dir(mojang.join(kind_dir(kind))).into_iter().flatten().flatten() {
            let folder = entry.file_name().to_string_lossy().to_string();
            // Talaz's own pack is the mods' and stays on
            if folder == "talaz" || folder.starts_with('.') {
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
                kind,
                shader: kind == "resource" && path.join("renderer/materials").is_dir(),
                name: plain(h["name"].as_str().unwrap_or(&folder)),
                description: plain(h["description"].as_str().unwrap_or_default()),
                on: kind == "resource" && on.contains(&uuid),
                uuid,
                folder,
            });
        }
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

/// What a pack is from its manifest's modules: "resource", "behavior" for data and scripts,
/// or None for what Talaz doesn't install (skin packs, world templates)
fn kind_of(dir: &Path) -> Option<&'static str> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    let types: Vec<&str> = manifest["modules"].as_array()?.iter().filter_map(|m| m["type"].as_str()).collect();
    if types.iter().any(|t| *t == "data" || *t == "script" || *t == "javascript") {
        Some("behavior")
    } else if types.contains(&"resources") {
        Some("resource")
    } else {
        None
    }
}

/// The folders in an unpacked file that hold a pack, the zipped packs inside it unpacked too
/// (an .mcaddon is a zip of .mcpacks or of pack folders)
fn pack_roots(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if dir.join("manifest.json").exists() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth > 3 {
        return;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let zipped = path.extension().is_some_and(|x| x == "mcpack" || x == "zip");
        if path.is_dir() {
            pack_roots(&path, depth + 1, out);
        } else if zipped {
            let inner = path.with_extension("unpacked");
            if let Ok(mut zip) = std::fs::File::open(&path).map_err(|e| e.to_string()).and_then(|f| zip::ZipArchive::new(f).map_err(|e| e.to_string())) {
                // enclosed_name refuses paths that would land outside the folder
                if zip.extract(&inner).is_ok() {
                    pack_roots(&inner, depth + 1, out);
                }
            }
        }
    }
}

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Unpacks a .mcpack, .mcaddon or .zip and puts each pack in it in its own folder of the
/// right kind. Resource packs are turned on; the behavior packs wait for a world to use them
pub fn install_file(path: &str) -> Result<Vec<(&'static str, String)>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "That isn't a pack file")?;
    let mojang = mojang()?;
    let staging = mojang.join("resource_packs").join(format!(".import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    zip.extract(&staging).map_err(|e| e.to_string())?;
    let mut roots = Vec::new();
    pack_roots(&staging, 0, &mut roots);
    let stem = safe_name(&Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or("pack".into()));
    let mut placed = Vec::new();
    let mut failed = None;
    for root in &roots {
        let (Some(kind), Some(h)) = (kind_of(root), header(root)) else { continue };
        let dir = mojang.join(kind_dir(kind));
        let _ = std::fs::create_dir_all(&dir);
        let base = if roots.len() > 1 { safe_name(&plain(h["name"].as_str().unwrap_or(&stem))) } else { stem.clone() };
        let mut dest = dir.join(&base);
        for i in 2.. {
            if !dest.exists() {
                break;
            }
            dest = dir.join(format!("{base} {i}"));
        }
        match std::fs::rename(root, &dest) {
            Ok(()) => placed.push((kind, dest.file_name().unwrap().to_string_lossy().to_string())),
            Err(e) => failed = Some(e.to_string()),
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    if placed.is_empty() {
        return Err(failed.unwrap_or_else(|| "No resource pack or add-on in it".to_string()));
    }
    for (kind, folder) in &placed {
        if *kind == "resource" {
            set_pack(folder.clone(), true)?;
        }
    }
    Ok(placed)
}

/// A dropped or picked file: what it held, for the message
#[tauri::command(async)]
pub fn import_pack(path: String) -> Result<String, String> {
    let placed = install_file(&path)?;
    let behavior = placed.iter().filter(|(k, _)| *k == "behavior").count();
    Ok(if behavior > 0 { "The add-on was added. Turn its behavior pack on in a world's settings".into() } else { "The pack was added and turned on".into() })
}

#[tauri::command]
pub fn remove_pack(folder: String, kind: Option<String>) -> Result<(), String> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') || folder == "talaz" {
        return Err("Not a pack".into());
    }
    let kind = kind.unwrap_or_default();
    if kind != "behavior" {
        set_pack(folder.clone(), false).ok();
    }
    std::fs::remove_dir_all(mojang()?.join(kind_dir(&kind)).join(folder)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An .mcaddon as CurseForge has them: a behavior pack folder, and a resource pack zipped
    /// inside as an .mcpack. Both are found and told apart; a skin pack is left out
    #[test]
    fn finds_the_packs_in_an_addon() {
        let dir = std::env::temp_dir().join(format!("talaz-addon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let manifest = |t: &str| format!(r#"{{"format_version":2,"header":{{"name":"x","uuid":"u"}},"modules":[{{"type":"{t}"}}]}}"#);
        std::fs::create_dir_all(dir.join("Mobs BP")).unwrap();
        std::fs::write(dir.join("Mobs BP/manifest.json"), manifest("data")).unwrap();
        std::fs::create_dir_all(dir.join("skins")).unwrap();
        std::fs::write(dir.join("skins/manifest.json"), manifest("skin_pack")).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join("Mobs RP.mcpack")).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(manifest("resources").as_bytes()).unwrap();
        zip.finish().unwrap();

        let mut roots = Vec::new();
        pack_roots(&dir, 0, &mut roots);
        let mut kinds: Vec<_> = roots.iter().map(|r| kind_of(r)).collect();
        kinds.sort();
        assert_eq!(kinds, vec![None, Some("behavior"), Some("resource")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
