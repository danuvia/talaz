#pragma once

#include "imgui.h"

// Tide's colors, shared by the menu and the HUDs
namespace theme {

constexpr ImU32 DEEP = IM_COL32(12, 18, 23, 255), INK = IM_COL32(16, 23, 29, 255), LINE = IM_COL32(30, 42, 51, 255),
                TEXT = IM_COL32(232, 238, 242, 255), MUTED = IM_COL32(154, 169, 181, 255), ACCENT = IM_COL32(79, 216, 196, 255),
                WATER = IM_COL32(79, 216, 196, 56),
                // INK, let through a little so a HUD doesn't hide the world behind it
                PANEL = IM_COL32(16, 23, 29, 215);

}  // namespace theme
