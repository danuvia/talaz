// Entry point mcpelauncher calls into. It wires the game window's input and
// frame callbacks to the menu; everything drawn lives in menu.cpp.
#include <EGL/egl.h>
#include <dlfcn.h>

#include "menu.h"
#include "config.h"
#include "mods.h"

namespace {

struct GameWindow;

GameWindow* (*get_primary_window)();
void (*add_window_creation_callback)(void* user, void (*cb)(void* user));
void (*add_swap_buffers_callback)(void* user, void (*cb)(void* user, EGLDisplay, EGLSurface));
void (*add_keyboard_callback)(GameWindow*, void* user, bool (*cb)(void* user, int key, int action));
void (*add_mouse_button_callback)(GameWindow*, void* user, bool (*cb)(void* user, double x, double y, int button, int action));
void (*add_mouse_position_callback)(GameWindow*, void* user, bool (*cb)(void* user, double x, double y, bool relative));
bool (*is_mouse_locked)(GameWindow*);
void (*add_mouse_scroll_callback)(GameWindow*, void* user, bool (*cb)(void* user, double x, double y, double dx, double dy));

template <typename T>
void load(void* lib, T& fn, const char* name) {
    fn = reinterpret_cast<T>(dlsym(lib, name));
}

void on_window_created(void*) {
    GameWindow* window = get_primary_window();
    add_keyboard_callback(window, nullptr, [](void*, int key, int action) { return menu::key(key, action); });
    add_mouse_button_callback(window, nullptr, [](void*, double x, double y, int button, int action) {
        return menu::mouse_button(x, y, button, action);
    });
    add_mouse_position_callback(window, nullptr, [](void*, double x, double y, bool relative) {
        return menu::mouse_move(x, y, relative);
    });
    add_mouse_scroll_callback(window, nullptr, [](void*, double, double, double dx, double dy) {
        return menu::scroll(dx, dy);
    });
}

}  // namespace

extern "C" [[gnu::visibility("default")]] void mod_preinit() {}

extern "C" [[gnu::visibility("default")]] void mod_init() {
    // every setting is declared before the file is read, and before input starts
    // Right Shift, as the launcher reports it
    config::declare("key.menu", 16 | 256);
    mods::declare();
    config::load();
    void* gw = dlopen("libmcpelauncher_gamewindow.so", 0);
    if (!gw)
        return;
    load(gw, get_primary_window, "game_window_get_primary_window");
    load(gw, add_window_creation_callback, "game_window_add_window_creation_callback");
    load(gw, add_swap_buffers_callback, "game_window_add_swap_buffers_callback");
    load(gw, add_keyboard_callback, "game_window_add_keyboard_callback");
    load(gw, add_mouse_button_callback, "game_window_add_mouse_button_callback");
    load(gw, add_mouse_position_callback, "game_window_add_mouse_position_callback");
    load(gw, add_mouse_scroll_callback, "game_window_add_mouse_scroll_callback");
    load(gw, is_mouse_locked, "game_window_is_mouse_locked");
    if (!get_primary_window || !add_window_creation_callback || !add_swap_buffers_callback || !add_keyboard_callback ||
        !add_mouse_button_callback || !add_mouse_position_callback || !add_mouse_scroll_callback)
        return;

    // Settings::menubarsize lives in the launcher itself, outside the Android
    // side mods see, so it is looked up through the launcher's own dlsym
    const int* menubar = nullptr;
    if (void* api = dlopen("libmcpelauncher_mod.so", 0)) {
        void* (*host_dlopen)(const char*, int) = nullptr;
        void* (*host_dlsym)(void*, const char*) = nullptr;
        load(api, host_dlopen, "mcpelauncher_host_dlopen");
        load(api, host_dlsym, "mcpelauncher_host_dlsym");
        if (host_dlopen && host_dlsym)
            if (void* launcher = host_dlopen(nullptr, RTLD_NOW))
                menubar = static_cast<const int*>(host_dlsym(launcher, "_ZN8Settings11menubarsizeE"));
    }
    menu::connect(is_mouse_locked ? +[] { return is_mouse_locked(get_primary_window()); } : nullptr, menubar);

    mods::install_zoom();

    add_window_creation_callback(nullptr, on_window_created);
    add_swap_buffers_callback(nullptr, [](void*, EGLDisplay display, EGLSurface surface) { menu::frame(display, surface); });
}
