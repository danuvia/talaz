//! The program that runs the game. On a Mac, Talaz downloads the mcpelauncher
//! build once and keeps it as its own app, so the game shows up as Talaz.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use tauri::{AppHandle, Emitter};

const DMG_URL: &str = "https://github.com/minecraft-linux/macos-builder/releases/download/v1.8.5/Minecraft.Bedrock.Launcher.dmg";
const DMG_SHA256: &str = "1faa08ed77f998a4c049b597300525761c5543209de5e51199ab32aa611197f4";
const DMG_APP: &str = "Minecraft Bedrock Launcher.app";

static INSTALLING: Mutex<()> = Mutex::new(());

/// Tells the page what a long step is doing; part is 0 to 1, or nothing when it can't be measured
pub fn progress(app: &AppHandle, text: &str, part: Option<f64>) {
    let _ = app.emit("progress", (text, part));
}

fn bundle() -> Option<PathBuf> {
    Some(crate::root()?.join("runtime/Talaz.app"))
}

/// One of the runtime's programs, like mcpelauncher-client
// ponytail: Linux takes them from PATH. Download a build there too once Talaz ships for Linux
pub fn tool(name: &str) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        let path = bundle()?.join("Contents/MacOS").join(name);
        return path.exists().then_some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|path| path.exists())
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|err| format!("{program}: {err}"))?;
    if !out.status.success() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Saves a download to a file, reporting how far it is
pub fn download(app: &AppHandle, text: &str, mut from: impl Read, to: &Path, done: &mut u64, total: u64) -> Result<(), String> {
    let mut file = std::fs::File::create(to).map_err(|err| err.to_string())?;
    let mut buf = vec![0; 1 << 16];
    let mut shown = u64::MAX;
    loop {
        let n = from.read(&mut buf).map_err(|err| format!("The download stopped: {err}"))?;
        if n == 0 {
            return Ok(());
        }
        file.write_all(&buf[..n]).map_err(|err| err.to_string())?;
        *done += n as u64;
        // one message per megabyte is plenty for a progress bar
        if *done >> 20 != shown {
            shown = *done >> 20;
            progress(app, text, (total > 0).then(|| (*done as f64 / total as f64).min(1.0)));
        }
    }
}

/// Makes sure the runtime is there, downloading it on first use
pub fn ensure(app: &AppHandle) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return tool("mcpelauncher-client").map(drop).ok_or("Install mcpelauncher-client first".into());
    }
    let _guard = INSTALLING.lock().map_err(|err| err.to_string())?;
    let bundle = bundle().ok_or("No home folder")?;
    if bundle.exists() {
        return Ok(());
    }
    let dir = bundle.parent().ok_or("No runtime folder")?;
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let (dmg, mount, part) = (dir.join("runtime.dmg"), dir.join("mount"), dir.join("Talaz.app.part"));

    let res = ureq::get(DMG_URL).call().map_err(|err| format!("Could not download the game runtime: {err}"))?;
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    download(app, "Downloading the game runtime", res.into_body().into_reader(), &dmg, &mut 0, total)?;

    progress(app, "Installing the game runtime", None);
    let sum = run("/usr/bin/shasum", &["-a".as_ref(), "256".as_ref(), dmg.as_os_str()])?;
    if sum.split_whitespace().next() != Some(DMG_SHA256) {
        let _ = std::fs::remove_file(&dmg);
        return Err("The downloaded runtime is not the expected file".into());
    }
    let _ = std::fs::remove_dir_all(&part);
    run("/usr/bin/hdiutil", &["attach".as_ref(), "-nobrowse".as_ref(), "-readonly".as_ref(), "-noautoopen".as_ref(), "-mountpoint".as_ref(), mount.as_os_str(), dmg.as_os_str()])?;
    let copied = run("/usr/bin/ditto", &[mount.join(DMG_APP).as_os_str(), part.as_os_str()]);
    let _ = run("/usr/bin/hdiutil", &["detach".as_ref(), "-force".as_ref(), mount.as_os_str()]);
    copied?;
    let _ = std::fs::remove_file(&dmg);

    // the bundle's name is what macOS shows in the menu bar and the Dock
    let plist = part.join("Contents/Info.plist");
    for (key, value) in [
        ("CFBundleName", "Talaz"),
        ("CFBundleGetInfoString", "Talaz"),
        ("CFBundleIdentifier", "dev.danuvia.talaz.game"),
        ("CFBundleExecutable", "mcpelauncher-client"),
    ] {
        run("/usr/bin/plutil", &["-replace".as_ref(), key.as_ref(), "-string".as_ref(), value.as_ref(), plist.as_os_str()])?;
    }
    let png = dir.join("icon.png");
    std::fs::write(&png, include_bytes!("../icons/game.png")).map_err(|err| err.to_string())?;
    // sips can't write icns, so build an iconset for iconutil. A missing icon is not worth failing the install over
    let set = dir.join("game.iconset");
    let _ = std::fs::create_dir_all(&set);
    for size in [16, 32, 128, 256, 512] {
        for (scale, name) in [(1, format!("icon_{size}x{size}.png")), (2, format!("icon_{size}x{size}@2x.png"))] {
            let px = (size * scale).to_string();
            let _ = run("/usr/bin/sips", &["-z".as_ref(), px.as_ref(), px.as_ref(), png.as_os_str(), "--out".as_ref(), set.join(name).as_os_str()]);
        }
    }
    let _ = run("/usr/bin/iconutil", &["-c".as_ref(), "icns".as_ref(), set.as_os_str(), "-o".as_ref(), part.join("Contents/Resources/minecraft.icns").as_os_str()]);
    let _ = std::fs::remove_dir_all(&set);
    let _ = std::fs::remove_file(&png);

    std::fs::rename(&part, &bundle).map_err(|err| err.to_string())
}
