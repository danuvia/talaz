# Talaz

A Minecraft Bedrock client for macOS and Linux, by Danuvia.

Talaz runs the Android build of Bedrock through [mcpelauncher](https://github.com/minecraft-linux/mcpelauncher-manifest). It adds its own title screen and an in-game menu with mods, and it updates itself. You need to own Minecraft on Google Play.

## Installing on a Mac

Download the `.dmg` for your Mac from [Releases](https://github.com/danuvia/talaz/releases): `aarch64` for Apple silicon, `x64` for Intel. Open it and drag Talaz into Applications.

Talaz isn't signed with an Apple certificate yet, so macOS won't open it the first time. Right click Talaz in Applications, choose Open, then Open again in the dialog. If the dialog only offers Done, open System Settings, go to Privacy & Security and click Open Anyway next to the message about Talaz. You only do this once.

On first start Talaz asks you to sign in to Google Play and pick a folder for your worlds. It downloads the game runtime and the version you choose, and Play starts it. New versions of Talaz install themselves when the launcher starts.

## What's in it

Turn mods on from the launcher's Mods page, or in a world with Right Shift. Each mod has its own page of settings. Riptide removes a frame cap mcpelauncher applies on Macs, which about triples the frame rate. Motion Blur, Camera Sway and Night Vision change how the world looks, Zoom, Toggle Sprint and FPS Limit change how it plays, and FPS, CPS, Clock, Session and Keystrokes are HUDs. Every mod and setting is listed in [`mods.def`](https://github.com/danuvia/talaz-mods/blob/main/src/mods.def).

## Layout

`app/` is the launcher, a Tauri app. It downloads the game, starts it with the mods loaded and puts the title screen's resource pack in place.

`mod/` is the mods, a C++ library mcpelauncher loads next to the game. They have their own repository, [talaz-mods](https://github.com/danuvia/talaz-mods), checked out here as a submodule. Clone with `git clone --recurse-submodules`, or run `git submodule update --init` after cloning.

## Building

You need CMake, the Android NDK and Rust. Build the mods first, because the launcher carries them inside itself.

```bash
cmake -S mod -B mod/build/arm64 -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 -DANDROID_STL=c++_static -DCMAKE_BUILD_TYPE=Release
```

```bash
cmake --build mod/build/arm64 -j8
```

On an Intel Mac or on Linux use `-DANDROID_ABI=x86_64` and `mod/build/x86_64`. Then start the launcher:

```bash
cd app/src-tauri && cargo run
```

On Linux, install `mcpelauncher-client` and `mcpelauncher-extract` yourself for now. `cargo tauri build` makes the `.app` and the `.dmg`.

## Releases

Pushing a tag like `v0.2.0` runs the release workflow. It builds Talaz for both kinds of Mac and leaves a draft release with the DMGs, the update archives and `latest.json`, which running launchers check at start. The updates are signed with the `TAURI_SIGNING_PRIVATE_KEY` secret.

## License

GPL-3.0. See [ATTRIBUTION.md](ATTRIBUTION.md) for the work Talaz builds on. Talaz is not affiliated with Mojang or Microsoft.
