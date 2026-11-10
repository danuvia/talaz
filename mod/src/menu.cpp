#include "menu.h"

#include <algorithm>
#include <atomic>
#include <chrono>

#include "assets.h"
#include "config.h"
#include "gl.h"
#include "imgui.h"
#include "imgui_impl_opengl3.h"

EMBED(font_regular, FONT_DIR "/Poppins-Regular.ttf")

namespace menu {
namespace {

// Key codes and actions as the launcher's game window reports them
constexpr int KEY_ESCAPE = 27;
constexpr int KEY_PRESS = 0;
constexpr int BUTTON_PRESS = 0;

// What the player asked for. Input arrives outside the frame, so the frame
// picks the change up from here.
std::atomic<bool> wanted{false};
bool ready = false;
float width = 0, height = 0, scale = 0;
// While the game holds the mouse it only sends relative motion, so the menu
// keeps its own cursor and draws it
float cursor_x = 0, cursor_y = 0;
bool relative_mouse = false;
std::chrono::steady_clock::time_point last_frame;
bool (*mouse_locked)() = nullptr;
const int* menubar_height = nullptr;

// A world is being played with no screen over it, the game holding the mouse
bool playing() { return !mouse_locked || mouse_locked(); }

// The launcher's window coordinates put the game's top edge below its menubar
float window_y(double y) { return float(y) - (menubar_height ? float(*menubar_height) : 0.f); }

bool init() {
    if (!gl::load())
        return false;
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();
    io.IniFilename = nullptr;
    io.ConfigFlags |= ImGuiConfigFlags_NoMouseCursorChange;
    ImGui_ImplOpenGL3_Init("#version 300 es");
    ready = true;
    return true;
}

void load_fonts() {
    ImGuiIO& io = ImGui::GetIO();
    io.Fonts->Clear();
    ImFontConfig config;
    config.FontDataOwnedByAtlas = false;
    Asset font = ASSET(font_regular);
    io.Fonts->AddFontFromMemoryTTF((void*)font.data, int(font.size), 16 * scale, &config);
    ImGui_ImplOpenGL3_DestroyFontsTexture();
    ImGui_ImplOpenGL3_CreateFontsTexture();
}

// Stands in until the menu is designed
void draw(ImVec2 size) {
    ImDrawList* dl = ImGui::GetForegroundDrawList();
    dl->AddRectFilled(ImVec2(0, 0), size, IM_COL32(0, 0, 0, 140));
    ImVec2 text = ImGui::CalcTextSize(CLIENT_NAME);
    dl->AddText(ImVec2((size.x - text.x) / 2, (size.y - text.y) / 2), IM_COL32_WHITE, CLIENT_NAME);
}

}  // namespace

void connect(bool (*locked)(), const int* menubar) {
    mouse_locked = locked;
    menubar_height = menubar;
}

bool key(int key, int action) {
    // The menu key opens the menu in a world and not over other screens; the
    // game only holds the mouse while nothing is open
    if (action == KEY_PRESS && key == int(config::get("key.menu")) && (wanted || playing())) {
        wanted = !wanted;
        return true;
    }
    if (wanted && action == KEY_PRESS && key == KEY_ESCAPE) {
        wanted = false;
        return true;
    }
    return wanted;
}

bool mouse_button(double x, double y, int button, int action) {
    if (!wanted || !ready)
        return wanted;
    // The launcher numbers buttons from 1, left first
    int index = button - 1;
    if (index >= 0 && index < 3) {
        if (!relative_mouse) {
            cursor_x = float(x);
            cursor_y = window_y(y);
        }
        ImGuiIO& io = ImGui::GetIO();
        io.AddMousePosEvent(cursor_x, cursor_y);
        io.AddMouseButtonEvent(index, action == BUTTON_PRESS);
    }
    return true;
}

bool mouse_move(double x, double y, bool relative) {
    relative_mouse = relative;
    if (relative && wanted) {
        cursor_x = std::clamp(cursor_x + float(x), 0.f, width);
        cursor_y = std::clamp(cursor_y + float(y), 0.f, height);
    } else if (!relative) {
        cursor_x = float(x);
        cursor_y = window_y(y);
    }
    if (wanted && ready)
        ImGui::GetIO().AddMousePosEvent(cursor_x, cursor_y);
    return wanted;
}

bool scroll(double dx, double dy) {
    if (wanted && ready)
        ImGui::GetIO().AddMouseWheelEvent(float(dx), float(dy));
    return wanted;
}

void frame(EGLDisplay display, EGLSurface surface) {
    auto now = std::chrono::steady_clock::now();
    float dt = last_frame.time_since_epoch().count() ? std::chrono::duration<float>(now - last_frame).count() : 1 / 60.f;
    dt = std::clamp(dt, 1 / 1000.f, 0.25f);
    last_frame = now;
    if (!wanted)
        return;

    EGLint w = 0, h = 0;
    eglQuerySurface(display, surface, EGL_WIDTH, &w);
    eglQuerySurface(display, surface, EGL_HEIGHT, &h);
    if (w <= 0 || h <= 0)
        return;
    width = float(w);
    height = float(h);
    if (!ready && !init()) {
        wanted = false;
        return;
    }

    ImVec2 size(width, height);
    // sized for a 1080p window
    float s = std::clamp(height / 1080, 0.5f, 3.f);
    if (s != scale) {
        scale = s;
        load_fonts();
    }

    ImGuiIO& io = ImGui::GetIO();
    io.DisplaySize = size;
    io.DeltaTime = dt;
    io.MouseDrawCursor = relative_mouse;

    ImGui_ImplOpenGL3_NewFrame();
    ImGui::NewFrame();
    draw(size);
    ImGui::Render();
    ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
}

}  // namespace menu
