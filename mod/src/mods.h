#pragma once

#include <span>
#include <string>
#include <vector>

#include "imgui.h"

// The mods the menu lists and what they do in the game
namespace mods {

struct Mod {
    const char* id;
    const char* name;
    const char* note;
    float level = 0;  // how far the menu tile's water has risen, 0 to 1
};

std::span<Mod> all();
bool enabled(const char* id);
void toggle(const char* id);
struct Setting {
    const char* mod;
    const char* name;
    const char* key;
    const char* unit = "";
    float min = 0, max = 1, step = 0;
    bool is_switch() const { return step == 0; }
};
std::vector<const Setting*> settings_of(const char* id);
// How a value reads on the menu: "4×", "60 fps", "On"
std::string setting_text(const Setting& s, float value);
// Snaps a slider's value to its steps. Saving waits for the end of a drag
void set_setting(const Setting& s, float value, bool save = true);
// The mod's first setting for its tile ("4×"), empty when it has none or it is a switch
std::string option_text(const char* id);

// Declares every mod setting with config
void declare();

// Finds the game's camera so Zoom can change the field of view. Needs the
// game library loaded, so it runs from mod_init.
void install_zoom();
// Holds the frame back when FPS Limit is on. Called once per frame.
void limit_fps();

// Input the game sees, observed for the HUDs and Zoom. on_scroll returns true
// when Zoom took the scroll.
/// True when the game should not see the key
bool on_key(int key, int action, bool playing);
void on_mouse_button(int button, int action);
bool on_scroll(double dy, bool playing);

// Called once per frame, before drawing
void tick(float dt);
bool hud_visible();
// Night Vision and Motion Blur, drawn over the finished picture
bool effects_visible();
void draw_effects(ImDrawList* dl);
// u is the size of one design pixel, the design being drawn for a 720p screen
void draw_hud(ImDrawList* dl, float u, ImFont* font);

}  // namespace mods
