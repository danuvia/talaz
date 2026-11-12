//! Windows: the player installs Minecraft from the Microsoft Store or the Xbox app,
//! and Talaz starts it and loads the mod into it. Nothing here downloads the game.

use std::{
    io::{BufRead, BufReader, Seek, SeekFrom},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Emitter};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT},
    System::{
        Diagnostics::{
            Debug::WriteProcessMemory,
            ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
        },
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Memory::{VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE},
        Threading::{
            CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE,
            PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
        },
    },
};

use crate::RUNNING;

const GAME_EXE: &str = "Minecraft.Windows.exe";
/// What the mod is called beside its settings, and the log it writes there
const MOD_FILE: &str = "libtalaz_bedrock.dll";

/// Minecraft registers the minecraft: link when it installs, Store and Xbox app alike
pub fn installed() -> bool {
    Command::new("reg").args(["query", r"HKCR\minecraft"]).output().is_ok_and(|out| out.status.success())
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect()
}

fn game_pid() -> Option<u32> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(GAME_EXE) {
                found = Some(entry.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
        found
    }
}

/// LoadLibraryW on the mod, run inside the game. Returns the game's handle for waiting on
fn inject(pid: u32, dll: &Path) -> Result<HANDLE, String> {
    let path = wide(&dll.display().to_string());
    let bytes = path.len() * 2;
    unsafe {
        let access = PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ | PROCESS_SYNCHRONIZE;
        let game = OpenProcess(access, 0, pid);
        if game.is_null() {
            return Err("Talaz couldn't reach Minecraft".into());
        }
        let remote = VirtualAllocEx(game, std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        let written = !remote.is_null() && WriteProcessMemory(game, remote, path.as_ptr().cast(), bytes, std::ptr::null_mut()) != 0;
        let load = GetProcAddress(GetModuleHandleW(wide("kernel32.dll").as_ptr()), c"LoadLibraryW".as_ptr().cast());
        let thread = match (written, load) {
            (true, Some(load)) => CreateRemoteThread(game, std::ptr::null(), 0, Some(std::mem::transmute(load)), remote, 0, std::ptr::null_mut()),
            _ => std::ptr::null_mut(),
        };
        let mut loaded = 0;
        if !thread.is_null() {
            WaitForSingleObject(thread, 10_000);
            GetExitCodeThread(thread, &mut loaded);
            CloseHandle(thread);
        }
        if !remote.is_null() {
            VirtualFreeEx(game, remote, 0, MEM_RELEASE);
        }
        if loaded == 0 {
            CloseHandle(game);
            return Err("Minecraft didn't take Talaz's mod".into());
        }
        Ok(game)
    }
}

/// Where the Store installed the game, which holds the vanilla resource pack
fn install_dir() -> Option<PathBuf> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", "(Get-AppxPackage -Name Microsoft.MinecraftUWP).InstallLocation"])
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    path.is_dir().then_some(path)
}

/// Puts Talaz's title screen pack in place. The game keeps a folder per account under
/// Users as well as Shared, and which one reads global_resource_packs.json depends on
/// the build, so every one gets the pack
// ponytail: guessed from where players find their worlds. Narrow it down once a Windows game shows which folder it reads
fn install_title_pack() {
    let (Some(game), Some(users)) = (install_dir(), crate::game_dir().and_then(|shared| shared.parent().map(Path::to_path_buf))) else { return };
    for entry in std::fs::read_dir(users).into_iter().flatten().flatten() {
        if entry.path().is_dir() {
            let _ = crate::install_pack(&entry.path(), &game);
        }
    }
}

/// Starts Minecraft, waits for it, loads the mod and follows the mod's log until the game closes
pub fn play(app: AppHandle, mods: PathBuf) -> Result<(), String> {
    if !installed() {
        return Err("Install Minecraft from the Microsoft Store or the Xbox app first".into());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Minecraft is already running".into());
    }
    let dll = mods.join(MOD_FILE);
    install_title_pack();
    // a game already open gets the mod too
    if game_pid().is_none() {
        if let Err(err) = Command::new("cmd").args(["/c", "start", "", "minecraft:"]).spawn() {
            RUNNING.store(false, Ordering::SeqCst);
            return Err(err.to_string());
        }
    }
    let _ = app.emit("game", true);
    std::thread::spawn(move || {
        let start = Instant::now();
        let pid = loop {
            if let Some(pid) = game_pid() {
                break Some(pid);
            }
            if start.elapsed() > Duration::from_secs(90) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(500));
        };
        let result = pid.ok_or_else(|| "Minecraft didn't start".to_string()).and_then(|pid| {
            // the game sets up its window and graphics before the mod looks for them
            std::thread::sleep(Duration::from_secs(3));
            inject(pid, &dll)
        });
        match result {
            Ok(game) => {
                follow_log(&app, &dll.with_extension("log"), game);
                unsafe { CloseHandle(game) };
            }
            Err(err) => {
                let _ = app.emit("log", vec![err]);
                crate::runtime::progress(&app, "Talaz couldn't load into Minecraft, see the Log page", None);
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
        let _ = app.emit("game", false);
    });
    Ok(())
}

/// Sends the mod's log to the Log page as it grows, until the game exits
fn follow_log(app: &AppHandle, log: &Path, game: HANDLE) {
    let mut seen = 0;
    loop {
        let exited = unsafe { WaitForSingleObject(game, 300) } != WAIT_TIMEOUT;
        if let Ok(mut file) = std::fs::File::open(log) {
            if file.seek(SeekFrom::Start(seen)).is_ok() {
                let lines: Vec<String> = BufReader::new(&mut file).lines().map_while(Result::ok).collect();
                seen = file.stream_position().unwrap_or(seen);
                if !lines.is_empty() {
                    let _ = app.emit("log", lines);
                }
            }
        }
        if exited {
            return;
        }
    }
}
