//! Browse: Bedrock resource packs from CurseForge, installed with one press. The file always
//! comes from CurseForge itself, so the author's page counts the download

use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;

const API: &str = "https://api.curseforge.com/v1";
const BEDROCK: u32 = 78022;

#[derive(Serialize)]
pub struct Item {
    id: u64,
    name: String,
    author: String,
    summary: String,
    downloads: u64,
    logo: Option<String>,
    page: Option<String>,
    // false when the author turned off downloads outside CurseForge's own site
    installable: bool,
}

fn key() -> Result<&'static str, String> {
    option_env!("CURSEFORGE_KEY").ok_or_else(|| "This build of Talaz can't browse CurseForge".to_string())
}

fn get(path: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let mut req = ureq::get(format!("{API}{path}")).header("x-api-key", key()?).header("accept", "application/json");
    for (k, v) in query {
        req = req.query(*k, v);
    }
    let text = req.call().map_err(|e| format!("CurseForge didn't answer: {e}"))?.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The class id of Bedrock's resource packs, looked up once
fn packs_class() -> Result<u64, String> {
    static CLASS: OnceLock<u64> = OnceLock::new();
    if let Some(id) = CLASS.get() {
        return Ok(*id);
    }
    let classes = get("/categories", &[("gameId", BEDROCK.to_string()), ("classesOnly", "true".into())])?;
    let id = classes["data"]
        .as_array()
        .and_then(|list| list.iter().find(|c| c["slug"] == "texture-packs" || c["slug"] == "resource-packs"))
        .and_then(|c| c["id"].as_u64())
        .ok_or("CurseForge has no resource packs for Bedrock")?;
    Ok(*CLASS.get_or_init(|| id))
}

/// sort: popular, downloads, updated or name
#[tauri::command(async)]
pub fn browse(search: String, sort: String, page: u32) -> Result<Vec<Item>, String> {
    let field = match sort.as_str() { "downloads" => "6", "updated" => "3", "name" => "4", _ => "2" };
    let order = if sort == "name" { "asc" } else { "desc" };
    let data = get("/mods/search", &[
        ("gameId", BEDROCK.to_string()),
        ("classId", packs_class()?.to_string()),
        ("searchFilter", search),
        ("sortField", field.into()),
        ("sortOrder", order.into()),
        ("pageSize", "24".into()),
        ("index", (page * 24).to_string()),
    ])?;
    Ok(data["data"].as_array().into_iter().flatten().map(|m| Item {
        id: m["id"].as_u64().unwrap_or_default(),
        name: m["name"].as_str().unwrap_or_default().to_string(),
        author: m["authors"][0]["name"].as_str().unwrap_or_default().to_string(),
        summary: m["summary"].as_str().unwrap_or_default().to_string(),
        downloads: m["downloadCount"].as_f64().unwrap_or_default() as u64,
        logo: m["logo"]["thumbnailUrl"].as_str().map(str::to_string),
        page: m["links"]["websiteUrl"].as_str().map(str::to_string),
        installable: m["allowModDistribution"].as_bool() != Some(false),
    }).collect())
}

/// Downloads the pack's newest file from CurseForge and adds it like a dropped .mcpack
#[tauri::command(async)]
pub fn install_pack(id: u64) -> Result<String, String> {
    let files = get(&format!("/mods/{id}/files"), &[("pageSize", "10".into())])?;
    let file = files["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["downloadUrl"].is_string())
        .max_by_key(|f| f["fileDate"].as_str().unwrap_or_default().to_string())
        .ok_or("The author only allows downloads on CurseForge's site")?;
    let url = file["downloadUrl"].as_str().unwrap();
    let name = file["fileName"].as_str().unwrap_or("pack.mcpack");
    let dir = std::env::temp_dir().join(format!("talaz-pack-{id}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name.replace(['/', '\\'], "_"));
    let mut body = ureq::get(url).call().map_err(|e| format!("The download failed: {e}"))?.into_body();
    let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    std::io::copy(&mut body.as_reader(), &mut out).map_err(|e| e.to_string())?;
    drop(out);
    let result = super::packs::import_pack(path.to_string_lossy().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Opens the pack's CurseForge page in the browser
#[tauri::command]
pub fn open_page(url: String) -> Result<(), String> {
    if !url.starts_with("https://www.curseforge.com/") {
        return Err("Not a CurseForge page".into());
    }
    super::runtime::system(if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" }).arg(url).spawn().map(drop).map_err(|e| e.to_string())
}
