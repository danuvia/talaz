#include "menu.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <string>

#include "assets.h"
#include "config.h"
#include "gl.h"
#include "imgui.h"
#include "imgui_impl_opengl3.h"
#include "mods.h"
#include "theme.h"

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
    io.Fonts->AddFontFromMemoryTTF((void*)font.data, int(font.size), 24 * scale, &config);
    ImGui_ImplOpenGL3_DestroyFontsTexture();
    ImGui_ImplOpenGL3_CreateFontsTexture();
}

using namespace theme;
constexpr float PI = 3.14159265f;

// The swell: a wave line across the box from a to b, crest height amp
void wave(ImDrawList* dl, float x0, float x1, float y, float amp, float phase) {
    for (int i = 0; i <= 24; i++)
        dl->PathLineTo(ImVec2(x0 + (x1 - x0) * i / 24, y - amp * std::sin(phase + i * PI / 6)));
}

void draw(ImVec2 size, float dt) {
    ImDrawList* dl = ImGui::GetForegroundDrawList();
    ImFont* font = ImGui::GetFont();
    ImVec2 mouse = ImGui::GetIO().MousePos;
    // the design is drawn for a 720p screen
    float u = scale * 1.5f, time = float(ImGui::GetTime());
    dl->AddRectFilled(ImVec2(0, 0), size, IM_COL32(6, 10, 14, 189));

    float left = std::round((size.x - 960 * u) / 2), top = std::round(96 * u);
    wave(dl, left, left + 24 * u, top + 16 * u, 3 * u, 0);
    dl->PathStroke(ACCENT, 0, 2.4f * u);
    wave(dl, left, left + 24 * u, top + 26 * u, 3 * u, 0);
    dl->PathStroke(ACCENT, 0, 2.4f * u);
    dl->AddText(font, 22 * u, ImVec2(left + 36 * u, top + 6 * u), TEXT, "talaz");

    float gap = 12 * u, w = (960 * u - 3 * gap) / 4, h = 132 * u, r = 14 * u;
    int i = 0;
    for (mods::Mod& m : mods::all()) {
        ImVec2 a(std::round(left + (i % 4) * (w + gap)), std::round(top + 64 * u + (i / 4) * (h + gap)));
        ImVec2 b(a.x + std::round(w), a.y + std::round(h));
        i++;
        bool hover = mouse.x >= a.x && mouse.x < b.x && mouse.y >= a.y && mouse.y < b.y;
        if (hover && ImGui::IsMouseClicked(0))
            mods::toggle(m.id);
        bool on = mods::enabled(m.id);
        m.level += ((on ? 1.f : 0.f) - m.level) * std::min(1.f, dt * 10);

        dl->AddRectFilled(a, b, INK, r);
        float rise = m.level * 62 * u;
        if (rise > 1) {
            float corner = std::min(r, rise);
            dl->PathArcTo(ImVec2(b.x - corner, b.y - corner), corner, 0, PI / 2);
            dl->PathArcTo(ImVec2(a.x + corner, b.y - corner), corner, PI / 2, PI);
            wave(dl, a.x, b.x, b.y - rise, std::min(3 * u, rise / 4), time * 1.5f + i);
            dl->PathFillConcave(WATER);
        }
        dl->AddRect(a, b, on ? ACCENT : hover ? MUTED : LINE, r, 0, std::max(1.f, u));
        dl->AddText(font, 15 * u, ImVec2(a.x + 16 * u, a.y + 14 * u), TEXT, m.name);
        float note = 12.5f * u;
        ImVec2 ns = font->CalcTextSizeA(note, FLT_MAX, w - 32 * u, m.note);
        dl->AddText(font, note, ImVec2(a.x + 16 * u, b.y - 14 * u - ns.y), on ? TEXT : MUTED, m.note, nullptr, w - 32 * u);
    }

    float foot = top + 64 * u + (i + 3) / 4 * (h + gap) + 8 * u, small = 12.5f * u;
    dl->AddText(font, small, ImVec2(left, foot), MUTED, "Click a tile to turn it on.");
    std::string profile = "Profile: " + config::profile();
    ImVec2 ps = font->CalcTextSizeA(small, FLT_MAX, 0, profile.c_str());
    dl->AddText(font, small, ImVec2(left + 960 * u - ps.x, foot), MUTED, profile.c_str());
}

}  // namespace

void connect(bool (*locked)(), const int* menubar) {
    mouse_locked = locked;
    menubar_height = menubar;
    // for looking at the menu without entering a world
    wanted = std::getenv("TALAZ_PREVIEW") != nullptr;
}

bool key(int key, int action) {
    mods::on_key(key, action, playing() && !wanted);
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
    if (!wanted)
        mods::on_mouse_button(button, action);
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
    if (!wanted && mods::on_scroll(dy, playing()))
        return true;
    if (wanted && ready)
        ImGui::GetIO().AddMouseWheelEvent(float(dx), float(dy));
    return wanted;
}

void frame(EGLDisplay display, EGLSurface surface) {
    auto now = std::chrono::steady_clock::now();
    float dt = last_frame.time_since_epoch().count() ? std::chrono::duration<float>(now - last_frame).count() : 1 / 60.f;
    dt = std::clamp(dt, 1 / 1000.f, 0.25f);
    last_frame = now;
    mods::tick(dt);
    // the HUDs show over a world being played, not over the game's screens
    if (!wanted && !(mods::hud_visible() && playing()))
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
    io.MouseDrawCursor = relative_mouse && wanted;

    ImGui_ImplOpenGL3_NewFrame();
    ImGui::NewFrame();
    if (wanted)
        draw(size, dt);
    else
        mods::draw_hud(ImGui::GetForegroundDrawList(), scale * 1.5f, ImGui::GetFont());
    ImGui::Render();
    ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
}

}  // namespace menu
