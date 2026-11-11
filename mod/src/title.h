#pragma once

#include "imgui.h"

// Talaz's title screen. The launcher's resource pack lays the game's own buttons
// out invisibly on the same spots and marks the screen with a pixel; the mod sees
// the mark and draws over them. The clicks still land on the game's buttons, so
// they do what they always did, except Mods, which has no game button.
namespace title {

// Looks for the pack's mark in the frame the game just drew
bool showing();

// Draws the screen. Returns true when the mod menu should open.
bool draw(ImDrawList* dl, ImVec2 display, ImVec2 mouse, bool clicked, float time);

// Whether a click at this point lands on a button only the mod handles
bool owns(ImVec2 point, ImVec2 display);

}  // namespace title
