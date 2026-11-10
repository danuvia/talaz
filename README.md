# Talaz

A Minecraft Bedrock client for macOS and Linux, by Danuvia.

Talaz runs the Android build of Bedrock through [mcpelauncher](https://github.com/minecraft-linux/mcpelauncher-manifest) and adds an in-game menu to it. You need to own Minecraft on Google Play.

## Layout

- `mod/` is the in-game mod, a C++ library mcpelauncher loads next to the game. It draws with Dear ImGui. Right Shift opens the menu, which is a placeholder for now.

## Building the mod

You need CMake and the Android NDK.

```bash
cmake -S mod -B mod/build/arm64 -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-28 -DANDROID_STL=c++_static -DCMAKE_BUILD_TYPE=Release
```

```bash
cmake --build mod/build/arm64 -j8
```

Use `-DANDROID_ABI=x86_64` on Linux. The result is `libtalaz_bedrock.so`, which goes in mcpelauncher's `mods` folder.

## License

GPL-3.0. See [ATTRIBUTION.md](ATTRIBUTION.md) for the work Talaz builds on.
