cmake_minimum_required(VERSION 3.15)

project(talaz_bedrock CXX)

set(CMAKE_CXX_STANDARD 20)
# for clangd
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

include(FetchContent)

FetchContent_Declare(imgui
        GIT_REPOSITORY https://github.com/ocornut/imgui.git
        GIT_TAG v1.91.9
        GIT_SHALLOW TRUE)
FetchContent_MakeAvailable(imgui)

add_library(talaz_bedrock SHARED
        src/main.cpp
        src/menu.cpp
        src/mods.cpp
        src/config.cpp
        src/gl.cpp
        ${imgui_SOURCE_DIR}/imgui.cpp
        ${imgui_SOURCE_DIR}/imgui_draw.cpp
        ${imgui_SOURCE_DIR}/imgui_tables.cpp
        ${imgui_SOURCE_DIR}/imgui_widgets.cpp
        ${imgui_SOURCE_DIR}/backends/imgui_impl_opengl3.cpp)

target_include_directories(talaz_bedrock PRIVATE ${imgui_SOURCE_DIR} ${imgui_SOURCE_DIR}/backends)
# The game ships its own libc++_shared, so keep ours static and our symbols private
# The name the in-game menu shows
set(CLIENT_NAME "Talaz" CACHE STRING "Name the in-game menu shows")
target_compile_definitions(talaz_bedrock PRIVATE IMGUI_IMPL_OPENGL_ES3 IMGUI_DISABLE_DEMO_WINDOWS
        CLIENT_NAME="${CLIENT_NAME}"
        FONT_DIR="${CMAKE_CURRENT_SOURCE_DIR}/assets/fonts")
target_compile_options(talaz_bedrock PRIVATE -fvisibility=hidden)
# Only the two entry points leave the library, so our static libc++ never
# stands in for the game's own
# The launcher's linker skips packed RELR relocations, which leaves static
# constructors pointing nowhere, so keep relocations in the plain format
target_link_options(talaz_bedrock PRIVATE
        -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/exports.map
        -Wl,--pack-dyn-relocs=none)
set_source_files_properties(${imgui_SOURCE_DIR}/backends/imgui_impl_opengl3.cpp PROPERTIES
        COMPILE_OPTIONS "-include;${CMAKE_CURRENT_SOURCE_DIR}/src/gl.h")
target_link_libraries(talaz_bedrock PRIVATE EGL log)
