#pragma once

#include <EGL/egl.h>

// Every input handler returns true when the menu swallowed the event, so the
// game never sees clicks and keys meant for the menu.
namespace menu {

// What the menu needs from the launcher: whether the game holds the mouse,
// which is when a world is being played with no screen open, and the height of
// the launcher's own menubar, which its mouse coordinates still include
// (mcpelauncher only takes it off for the game). Either may be null.
void connect(bool (*mouse_locked)(), const int* menubar_height);

bool key(int key, int action);
bool mouse_button(double x, double y, int button, int action);
bool mouse_move(double x, double y, bool relative);
bool scroll(double dx, double dy);
void frame(EGLDisplay display, EGLSurface surface);

}  // namespace menu
