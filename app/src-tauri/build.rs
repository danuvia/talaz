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
    // the Lua mods' header lines, for the Mods page: each file's "--@" lines after a "--@@ file" line
    let scripts = std::path::Path::new("../../mod/scripts");
    println!("cargo:rerun-if-changed={}", scripts.display());
    let mut headers = String::new();
    let mut files: Vec<_> = std::fs::read_dir(scripts).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "lua")).collect();
    files.sort();
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        headers.push_str(&format!("--@@ {}\n", file.display()));
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines().filter(|l| l.starts_with("--@ ")) {
            headers.push_str(line);
            headers.push('\n');
        }
    }
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("scripts.txt"), headers).unwrap();
    tauri_build::build()
}
