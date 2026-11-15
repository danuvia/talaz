fn main() {
    // CurseForge's key for the Browse page: CURSEFORGE_KEY in CI, ~/.config/talaz/curseforge-key
    // on a developer's computer. Never in the repository
    println!("cargo:rerun-if-env-changed=CURSEFORGE_KEY");
    let file = std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config/talaz/curseforge-key"));
    if let Some(file) = &file {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let key = std::env::var("CURSEFORGE_KEY").ok().or_else(|| file.and_then(|f| std::fs::read_to_string(f).ok()));
    if let Some(key) = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        println!("cargo:rustc-env=CURSEFORGE_KEY={key}");
    }
    tauri_build::build()
}
