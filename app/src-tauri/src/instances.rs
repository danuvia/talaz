//! Instances, after OneClient's: a name, a game version, Talaz's mods or none, and a game
//! folder that is either shared (the one picked in the setup) or the instance's own. Each one
//! uses the mod profile of its name, keeps its own logs and play history, and lists its worlds.
//! The open instance is the selected one, so every page acts on it

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub description: String,
    /// empty until picked: the page offers the recommended version
    pub version: String,
    /// starts the game as it comes: no Talaz mods, no Talaz title screen
    pub vanilla: bool,
    /// the setup's game folder, which other shared instances use too, or a folder of its own
    pub shared: bool,
    /// the game window, 0 for the launcher's own setting
    pub width: u32,
    pub height: u32,
    /// (start, seconds played), newest last, the last 50
    pub sessions: Vec<(u64, u64)>,
}

#[derive(Serialize, Deserialize)]
pub struct Instances {
    pub selected: String,
    pub list: Vec<Instance>,
}

fn file() -> Option<PathBuf> {
    Some(super::root()?.join("instances.json"))
}

pub fn load() -> Instances {
    let saved = file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Instances>(&t).ok());
    let mut all = saved.unwrap_or(Instances { selected: "default".into(), list: vec![] });
    if !all.list.iter().any(|i| i.id == "default") {
        all.list.insert(0, Instance { id: "default".into(), name: "Default Profile".into(), shared: true, ..Default::default() });
    }
    if !all.list.iter().any(|i| i.id == all.selected) {
        all.selected = "default".into();
    }
    all
}

fn store(all: &Instances) -> Result<(), String> {
    let file = file().ok_or("No home folder")?;
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(file, serde_json::to_string_pretty(all).unwrap()).map_err(|e| e.to_string())
}

pub fn selected() -> Instance {
    let all = load();
    all.list.iter().find(|i| i.id == all.selected).cloned().unwrap()
}

/// An instance's own folder, used when it isn't shared
pub fn folder(id: &str) -> Option<PathBuf> {
    Some(super::root()?.join("instances").join(id))
}

/// Names become the mod's profile file names, so only plain characters
fn clean(name: &str) -> Result<String, String> {
    let name: String = name.trim().chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.')).collect();
    let name = name.trim_matches('.').trim().to_string();
    if name.is_empty() || name.len() > 32 {
        return Err("Give it a name of up to 32 letters".into());
    }
    Ok(name)
}

/// Points the mod at the instance's profile, so the Mods page and the game agree
fn use_profile(name: &str) -> Result<(), String> {
    let base = super::mods_dir().ok_or("No home folder")?.join("libtalaz_bedrock.ini");
    std::fs::create_dir_all(base.parent().unwrap()).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&base).unwrap_or_default();
    let mut out: String = text.lines().filter(|l| !l.starts_with("profile=")).map(|l| format!("{l}\n")).collect();
    out.push_str(&format!("profile={name}\n"));
    std::fs::write(base, out).map_err(|e| e.to_string())
}

/// Moves a file or folder to the system's trash, so a mistake can be undone
fn trash(path: &Path) -> Result<(), String> {
    use std::process::Command;
    let p = path.to_string_lossy().to_string();
    let ok = if cfg!(target_os = "macos") {
        Command::new("osascript").args(["-e", "on run argv", "-e", "tell application \"Finder\" to delete (POSIX file (item 1 of argv) as alias)", "-e", "end run", &p]).output()
    } else if cfg!(windows) {
        let kind = if path.is_dir() { "DeleteDirectory" } else { "DeleteFile" };
        Command::new("powershell").args(["-NoProfile", "-Command", &format!("Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::{kind}($args[0], 'OnlyErrorDialogs', 'SendToRecycleBin')"), &p]).output()
    } else {
        super::runtime::system("gio").args(["trash", &p]).output()
    }
    .map_err(|e| e.to_string())?;
    if ok.status.success() { Ok(()) } else { Err("Couldn't move it to the trash".into()) }
}

fn running() -> bool {
    super::RUNNING.load(std::sync::atomic::Ordering::SeqCst)
}

#[tauri::command]
pub fn instances() -> Instances {
    load()
}

#[tauri::command]
pub fn select_instance(id: String) -> Result<(), String> {
    let mut all = load();
    if all.selected != id && running() {
        return Err("Close the game before switching instances".into());
    }
    let name = all.list.iter().find(|i| i.id == id).ok_or("No such instance")?.name.clone();
    all.selected = id;
    use_profile(&name)?;
    store(&all)
}

#[derive(Deserialize)]
pub struct Spec {
    name: String,
    description: String,
    version: String,
    vanilla: bool,
    shared: bool,
}

#[tauri::command]
pub fn create_instance(spec: Spec) -> Result<String, String> {
    let name = clean(&spec.name)?;
    let mut all = load();
    if all.list.iter().any(|i| i.name.eq_ignore_ascii_case(&name)) {
        return Err("There's already an instance with that name".into());
    }
    let base: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    let mut id = base.clone();
    for n in 2.. {
        if id != "default" && !all.list.iter().any(|i| i.id == id) && !folder(&id).is_some_and(|f| f.exists()) {
            break;
        }
        id = format!("{base}-{n}");
    }
    // like OneClient, a vanilla instance always gets its own folder
    let shared = spec.shared && !spec.vanilla;
    if !shared {
        std::fs::create_dir_all(folder(&id).ok_or("No home folder")?).map_err(|e| e.to_string())?;
    }
    all.list.push(Instance { id: id.clone(), name: name.clone(), description: spec.description.trim().into(), version: spec.version, vanilla: spec.vanilla, shared, ..Default::default() });
    if !running() {
        all.selected = id.clone();
        use_profile(&name)?;
    }
    store(&all)?;
    Ok(id)
}

#[derive(Deserialize)]
pub struct Edit {
    id: String,
    name: String,
    description: String,
    version: String,
    vanilla: bool,
    shared: bool,
    width: u32,
    height: u32,
}

/// Saves the instance's settings. A rename moves its mod profile with it
#[tauri::command]
pub fn update_instance(edit: Edit) -> Result<(), String> {
    let name = clean(&edit.name)?;
    let mut all = load();
    if all.list.iter().any(|i| i.id != edit.id && i.name.eq_ignore_ascii_case(&name)) {
        return Err("There's already an instance with that name".into());
    }
    let selected = all.selected == edit.id;
    let instance = all.list.iter_mut().find(|i| i.id == edit.id).ok_or("No such instance")?;
    // the version and Talaz or vanilla are picked when the instance is made and stay; only an
    // instance that never had a version (the default one at first) gets to pick one
    let (edit_version, edit_vanilla) = (if instance.version.is_empty() { edit.version } else { instance.version.clone() }, instance.vanilla);
    if instance.shared != edit.shared && selected && running() {
        return Err("Close the game first".into());
    }
    if instance.name != name {
        let dir = super::mods_dir().ok_or("No home folder")?;
        let from = dir.join(format!("libtalaz_bedrock.profile.{}.ini", instance.name));
        if from.exists() {
            std::fs::rename(from, dir.join(format!("libtalaz_bedrock.profile.{name}.ini"))).map_err(|e| e.to_string())?;
        }
        instance.name = name.clone();
    }
    instance.description = edit.description.trim().into();
    instance.version = edit_version;
    instance.vanilla = edit_vanilla;
    // the default instance is the shared folder itself
    instance.shared = (edit.shared || instance.id == "default") && !edit_vanilla;
    if !instance.shared {
        std::fs::create_dir_all(folder(&instance.id).ok_or("No home folder")?).map_err(|e| e.to_string())?;
    }
    instance.width = edit.width;
    instance.height = edit.height;
    if selected {
        use_profile(&name)?;
    }
    store(&all)
}

/// Takes an instance off the list. With `files`, its own folder goes to the trash too;
/// a shared folder always stays, other instances use it
#[tauri::command]
pub fn delete_instance(id: String, files: bool) -> Result<(), String> {
    if id == "default" {
        return Err("The default instance stays".into());
    }
    let mut all = load();
    if all.selected == id && running() {
        return Err("Close the game before deleting this instance".into());
    }
    let instance = all.list.iter().find(|i| i.id == id).cloned().ok_or("No such instance")?;
    if files && !instance.shared {
        if let Some(dir) = folder(&id).filter(|d| d.exists()) {
            trash(&dir)?;
        }
    }
    all.list.retain(|i| i.id != id);
    if all.selected == id {
        all.selected = "default".into();
        use_profile(&all.list[0].name)?;
    }
    store(&all)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn start_time() -> u64 {
    now()
}

/// Adds a finished game to the instance's play history
pub fn record(id: &str, started: u64) {
    let mut all = load();
    if let Some(i) = all.list.iter_mut().find(|i| i.id == id) {
        i.sessions.push((started, now().saturating_sub(started)));
        let extra = i.sessions.len().saturating_sub(50);
        i.sessions.drain(..extra);
        let _ = store(&all);
    }
}

fn modified(path: &Path) -> u64 {
    path.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

fn size(path: &Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| size(&e.path())).sum()
    } else {
        path.metadata().map_or(0, |m| m.len())
    }
}

// --- worlds ---

#[derive(Serialize)]
pub struct World {
    folder: String,
    name: String,
    last_played: u64,
    size: u64,
}

fn worlds_dir() -> Result<PathBuf, String> {
    Ok(super::game_dir().ok_or("No home folder")?.join("games/com.mojang/minecraftWorlds"))
}

/// A world's folder name is from the page, so only a plain name inside the worlds folder
fn world_path(folder: &str) -> Result<PathBuf, String> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') {
        return Err("Not a world".into());
    }
    Ok(worlds_dir()?.join(folder))
}

#[tauri::command]
pub fn worlds() -> Result<Vec<World>, String> {
    let mut list: Vec<World> = std::fs::read_dir(worlds_dir()?)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("level.dat").exists())
        .map(|e| {
            let path = e.path();
            let folder = e.file_name().to_string_lossy().to_string();
            let name = std::fs::read_to_string(path.join("levelname.txt")).map(|n| n.trim().to_string()).unwrap_or_else(|_| folder.clone());
            World { name, last_played: modified(&path.join("level.dat")), size: size(&path), folder }
        })
        .collect();
    list.sort_by(|a, b| b.last_played.cmp(&a.last_played));
    Ok(list)
}

#[tauri::command]
pub fn delete_world(folder: String) -> Result<(), String> {
    if running() {
        return Err("Worlds can't be deleted while the game is running".into());
    }
    trash(&world_path(&folder)?)
}

#[tauri::command]
pub fn open_world(folder: String) -> Result<(), String> {
    open(&world_path(&folder)?)
}

fn open(path: &Path) -> Result<(), String> {
    super::runtime::system(if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(path).spawn().map(drop).map_err(|e| e.to_string())
}

// --- logs ---

#[derive(Serialize)]
pub struct LogFile {
    name: String,
    modified: u64,
    size: u64,
}

pub fn logs_dir() -> Option<PathBuf> {
    Some(super::game_dir()?.join("logs"))
}

/// Before a game starts: the last game's log gets a name of its own, and only the newest 20 stay
pub fn rotate(dir: &Path) {
    let latest = dir.join("latest.log");
    if latest.exists() {
        let _ = std::fs::rename(&latest, dir.join(format!("game-{}.log", modified(&latest))));
    }
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("game-"))).collect();
    old.sort_by_key(|p| std::cmp::Reverse(modified(p)));
    for path in old.iter().skip(20) {
        let _ = std::fs::remove_file(path);
    }
}

fn log_path(name: &str) -> Result<PathBuf, String> {
    if !name.ends_with(".log") || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("Not a log".into());
    }
    Ok(logs_dir().ok_or("No home folder")?.join(name))
}

#[tauri::command]
pub fn logs() -> Vec<LogFile> {
    let mut list: Vec<LogFile> = std::fs::read_dir(logs_dir().unwrap_or_default())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".log"))
        .map(|e| LogFile { name: e.file_name().to_string_lossy().to_string(), modified: modified(&e.path()), size: size(&e.path()) })
        .collect();
    list.sort_by(|a, b| b.modified.cmp(&a.modified));
    list
}

#[tauri::command]
pub fn read_log(name: String) -> Result<String, String> {
    let bytes = std::fs::read(log_path(&name)?).map_err(|_| "That log is gone".to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[tauri::command]
pub fn delete_log(name: String) -> Result<(), String> {
    if name == "latest.log" && running() {
        return Err("The running game is writing that one".into());
    }
    std::fs::remove_file(log_path(&name)?).map_err(|e| e.to_string())
}
