#include "menu.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <string>

#include "assets.h"
#include "config.h"
#include "gl.h"
#include "imgui.h"
#include "imgui_impl_opengl3.h"
#include "mods.h"
#include "theme.h"
#include "title.h"

EMBED(font_regular, FONT_DIR "/Poppins-Regular.ttf")

namespace menu {
namespace {

// Key codes and actions as the launcher's game window reports them
constexpr int KEY_ESCAPE = 27;
constexpr int KEY_PRESS = 0, KEY_RELEASE = 2;
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
// Talaz's title screen is up, seen in the last frame, and a click on a button only the mod handles
std::atomic<bool> on_title{false}, title_click{false};
// The slider being dragged on a settings page
const mods::Setting* dragging = nullptr;
// The mod whose settings page is open, none for the grid of tiles
std::atomic<mods::Mod*> page{nullptr};
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
    // the title screen's wordmark, drawn large, gets a size of its own so it stays sharp
    io.Fonts->AddFontFromMemoryTTF((void*)font.data, int(font.size), 108 * scale, &config);
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

bool inside(ImVec2 p, ImVec2 a, ImVec2 b) { return p.x >= a.x && p.x < b.x && p.y >= a.y && p.y < b.y; }

// An on and off switch drawn at a, its knob sliding over with level 0 to 1
void toggle_switch(ImDrawList* dl, ImVec2 a, float u, bool on) {
    ImVec2 b(a.x + 44 * u, a.y + 24 * u);
    dl->AddRectFilled(a, b, on ? ACCENT : LINE, 12 * u);
    dl->AddCircleFilled(ImVec2(on ? b.x - 12 * u : a.x + 12 * u, a.y + 12 * u), 9 * u, on ? DEEP : MUTED);
}

// One mod's own page: a way back, its switch and every setting it has
void draw_page(ImDrawList* dl, mods::Mod& m, float left, float top, float u) {
    ImFont* font = ImGui::GetFont();
    ImVec2 mouse = ImGui::GetIO().MousePos;
    bool click = ImGui::IsMouseClicked(0);
    float width = 960 * u;

    // back to the tiles
    ImVec2 back(left, top + 56 * u);
    ImVec2 back_end(left + 90 * u, back.y + 24 * u);
    bool over_back = inside(mouse, back, back_end);
    dl->PathLineTo(ImVec2(back.x + 8 * u, back.y + 6 * u));
    dl->PathLineTo(ImVec2(back.x + 2 * u, back.y + 12 * u));
    dl->PathLineTo(ImVec2(back.x + 8 * u, back.y + 18 * u));
    dl->PathStroke(over_back ? TEXT : MUTED, 0, 2 * u);
    dl->AddText(font, 14 * u, ImVec2(back.x + 16 * u, back.y + 3 * u), over_back ? TEXT : MUTED, "Mods");
    if (over_back && click)
        page = nullptr;

    dl->AddText(font, 30 * u, ImVec2(left, top + 92 * u), TEXT, m.name);
    dl->AddText(font, 14 * u, ImVec2(left, top + 134 * u), MUTED, m.note);

    // the rows sit in one panel, the mod's own switch first
    auto settings = mods::settings_of(m.id);
    float row = 58 * u, y = top + 174 * u;
    ImVec2 pa(left, y), pb(left + width, y + row * (1 + settings.size()));
    dl->AddRectFilled(pa, pb, INK, 16 * u);
    dl->AddRect(pa, pb, LINE, 16 * u, 0, std::max(1.f, u));
    auto label = [&](float ry, const char* text) {
        dl->AddText(font, 15 * u, ImVec2(left + 20 * u, ry + (row - 15 * u) / 2 - 2 * u), TEXT, text);
    };
    auto switch_at = [&](float ry, bool on) {
        ImVec2 a(left + width - 64 * u, ry + (row - 24 * u) / 2);
        toggle_switch(dl, a, u, on);
        return click && inside(mouse, ImVec2(left, ry), ImVec2(left + width, ry + row));
    };

    label(y, "Enabled");
    if (switch_at(y, mods::enabled(m.id)))
        mods::toggle(m.id);
    for (const mods::Setting* s : settings) {
        y += row;
        dl->AddLine(ImVec2(left + 20 * u, y), ImVec2(left + width - 20 * u, y), LINE, std::max(1.f, u));
        label(y, s->name);
        float value = config::get(s->key);
        if (s->is_switch()) {
            if (switch_at(y, value != 0))
                mods::set_setting(*s, value != 0 ? 0 : 1);
            continue;
        }
        // a slider: drag along it, the value reads to its left
        float tw = 260 * u, tx = left + width - 28 * u - tw, ty = y + row / 2;
        float part = std::clamp((value - s->min) / (s->max - s->min), 0.f, 1.f);
        ImVec2 ta(tx - 10 * u, ty - 14 * u), tb(tx + tw + 10 * u, ty + 14 * u);
        bool over = inside(mouse, ta, tb);
        if (over && click)
            dragging = s;
        if (dragging == s) {
            float at = std::clamp((mouse.x - tx) / tw, 0.f, 1.f);
            bool held = ImGui::IsMouseDown(0);
            mods::set_setting(*s, s->min + at * (s->max - s->min), !held);
            if (!held)
                dragging = nullptr;
            value = config::get(s->key);
            part = std::clamp((value - s->min) / (s->max - s->min), 0.f, 1.f);
        }
        dl->AddRectFilled(ImVec2(tx, ty - 2 * u), ImVec2(tx + tw, ty + 2 * u), LINE, 2 * u);
        dl->AddRectFilled(ImVec2(tx, ty - 2 * u), ImVec2(tx + tw * part, ty + 2 * u), ACCENT, 2 * u);
        dl->AddCircleFilled(ImVec2(tx + tw * part, ty), (over || dragging == s ? 9 : 8) * u, ACCENT);
        std::string text = mods::setting_text(*s, value);
        ImVec2 ts = font->CalcTextSizeA(14 * u, FLT_MAX, 0, text.c_str());
        dl->AddText(font, 14 * u, ImVec2(tx - 20 * u - ts.x, ty - ts.y / 2), ACCENT, text.c_str());
    }
    if (settings.empty())
        dl->AddText(font, 13 * u, ImVec2(left, pb.y + 14 * u), MUTED, "This mod has nothing to set besides on and off.");
}

// The water of the launcher's tiles: a spring moves the level so it sloshes and
// settles, and turning a mod on sends a wave in from the right. It moves every
// frame, menu open or not, so leaving the menu mid-wave doesn't freeze it
void step_water(float dt) {
    for (mods::Mod& m : mods::all()) {
        bool on = mods::enabled(m.id);
        // the first look settles the water where it belongs, without a wave
        if (!m.seen)
            m.seen = true, m.was_on = on, m.level = on ? 0.4f : -0.2f;
        if (on && !m.was_on)
            m.cover = 0, m.level = 0.9f, m.vel = 0;
        m.was_on = on;
        if (m.cover < 1)
            m.cover = std::min(1.f, m.cover + dt / 0.45f);
        float target = !on ? -0.2f : m.cover < 1 ? 0.92f : 0.4f;
        m.vel += ((target - m.level) * 60 - m.vel * 8) * dt;
        m.level += m.vel * dt;
    }
}

void draw(ImVec2 size, float dt) {
    ImDrawList* dl = ImGui::GetForegroundDrawList();
    ImFont* font = ImGui::GetFont();
    ImVec2 mouse = ImGui::GetIO().MousePos;
    // the design is drawn for a 720p screen, and shrinks to fit a narrow window
    float u = std::min(scale * 1.5f, size.x / 1024), time = float(ImGui::GetTime());
    dl->AddRectFilled(ImVec2(0, 0), size, IM_COL32(6, 10, 14, 189));

    float left = std::round((size.x - 960 * u) / 2), top = std::round(96 * u);
    wave(dl, left, left + 24 * u, top + 16 * u, 3 * u, 0);
    dl->PathStroke(ACCENT, 0, 2.4f * u);
    wave(dl, left, left + 24 * u, top + 26 * u, 3 * u, 0);
    dl->PathStroke(ACCENT, 0, 2.4f * u);
    dl->AddText(font, 22 * u, ImVec2(left + 36 * u, top + 6 * u), TEXT, "talaz");

    if (mods::Mod* open = page) {
        draw_page(dl, *open, left, top, u);
        return;
    }

    float gap = 12 * u, w = (960 * u - 3 * gap) / 4, h = 132 * u, r = 14 * u;
    int i = 0;
    for (mods::Mod& m : mods::all()) {
        ImVec2 a(std::round(left + (i % 4) * (w + gap)), std::round(top + 64 * u + (i / 4) * (h + gap)));
        ImVec2 b(a.x + std::round(w), a.y + std::round(h));
        i++;
        bool hover = mouse.x >= a.x && mouse.x < b.x && mouse.y >= a.y && mouse.y < b.y;
        if (hover && ImGui::IsMouseClicked(0) && !inside(mouse, ImVec2(b.x - 40 * u, a.y + 8 * u), ImVec2(b.x, a.y + 36 * u)))
            mods::toggle(m.id);
        // three dots in the corner, or a right click, open the mod's own page
        ImVec2 dots(b.x - 26 * u, a.y + 22 * u);
        bool over_dots = inside(mouse, ImVec2(dots.x - 12 * u, dots.y - 12 * u), ImVec2(dots.x + 18 * u, dots.y + 12 * u));
        if ((over_dots && ImGui::IsMouseClicked(0)) || (hover && ImGui::IsMouseClicked(1))) {
            page = &m;
            continue;
        }
        bool on = mods::enabled(m.id);
        dl->AddRectFilled(a, b, INK, r);
        if (m.level > -0.08f) {
            // the inside of the rounded tile at x, so the water never leaves it
            auto edge = [&](float x) {
                float d = std::min(x - a.x, b.x - x);
                return d >= r ? 0.f : r - std::sqrt(std::max(0.f, r * r - (r - d) * (r - d)));
            };
            float H = b.y - a.y, W = b.x - a.x, amp = 2.5f * u + std::min(12 * u, std::abs(m.vel) * 16 * u);
            auto surface = [&](float x) {
                float y = b.y - H * m.level + amp * std::sin((x - a.x) / u * 0.04f + time * 2.4f + i) +
                          amp * 0.45f * std::sin((x - a.x) / u * 0.09f - time * 3.3f);
                return std::clamp(y, a.y + edge(x), b.y - edge(x));
            };
            float x0 = a.x + W * std::pow(1 - m.cover, 3.f), step = 4 * u;
            if (m.cover < 1) {
                // the front curls forward at the top
                float y0 = surface(x0);
                for (float y = b.y - edge(x0); y > y0; y -= step) {
                    float f = (b.y - y) / std::max(1.f, b.y - y0);
                    float x = std::max(a.x + edge(x0), x0 - 18 * u * f * f - 4 * u * std::sin(y / u * 0.15f + time * 10));
                    dl->PathLineTo(ImVec2(x, std::clamp(y, a.y + edge(x), b.y - edge(x))));
                }
            }
            for (float x = x0; x < b.x; x += step)
                dl->PathLineTo(ImVec2(x, surface(x)));
            dl->PathLineTo(ImVec2(b.x, surface(b.x)));
            for (float x = b.x; x > x0; x -= step)
                dl->PathLineTo(ImVec2(x, b.y - edge(x)));
            dl->PathLineTo(ImVec2(x0, b.y - edge(x0)));
            dl->PathFillConcave(WATER);
            // Riptide's current: streaks racing through its water while it's on
            if (on && m.cover >= 1 && std::strcmp(m.id, "riptide") == 0) {
                // in the band of water above the note, so they never cross its words
                float top = b.y - H * m.level + 5 * u, floor = b.y - 46 * u;
                for (int k = 0; k < 3; k++) {
                    float lane = top + (floor - top) * (0.15f + 0.35f * k), len = 26 * u + 9 * u * (k % 3);
                    float x = a.x + std::fmod(time * (220 + 70 * k) * u + k * 97 * u, W + len) - len;
                    float x1 = std::clamp(x, a.x + r, b.x - r), x2 = std::clamp(x + len, a.x + r, b.x - r);
                    if (x2 > x1 && lane < b.y - edge(x2) - 2 * u)
                        dl->AddLine(ImVec2(x1, lane), ImVec2(x2, lane), IM_COL32(79, 216, 196, 150), std::max(1.f, 1.6f * u));
                }
            }
        }
        dl->AddRect(a, b, on ? ACCENT : hover ? MUTED : LINE, r, 0, std::max(1.f, u));
        dl->AddText(font, 15 * u, ImVec2(a.x + 16 * u, a.y + 14 * u), TEXT, m.name);
        for (int d = 0; d < 3; d++)
            dl->AddCircleFilled(ImVec2(b.x - 32 * u + d * 6 * u, a.y + 22 * u), 1.8f * u, over_dots ? TEXT : MUTED);
        // the first setting under the name, so the tile tells what it is set to
        if (std::string opt = mods::option_text(m.id); !opt.empty())
            dl->AddText(font, 12.5f * u, ImVec2(a.x + 16 * u, a.y + 36 * u), on ? ACCENT : MUTED, opt.c_str());
        float note = 12.5f * u;
        ImVec2 ns = font->CalcTextSizeA(note, FLT_MAX, w - 32 * u, m.note);
        dl->AddText(font, note, ImVec2(a.x + 16 * u, b.y - 14 * u - ns.y), on ? TEXT : MUTED, m.note, nullptr, w - 32 * u);
    }

    float foot = top + 64 * u + (i + 3) / 4 * (h + gap) + 8 * u, small = 12.5f * u;
    dl->AddText(font, small, ImVec2(left, foot), MUTED, "Click a tile to turn it on. Its dots, or a right click, open its settings.");
    std::string profile = "Profile: " + config::profile();
    ImVec2 ps = font->CalcTextSizeA(small, FLT_MAX, 0, profile.c_str());
    dl->AddText(font, small, ImVec2(left + 960 * u - ps.x, foot), MUTED, profile.c_str());
}

}  // namespace

void connect(bool (*locked)(), const int* menubar) {
    mouse_locked = locked;
    menubar_height = menubar;
    // for looking at the menu without entering a world
    // TALAZ_PREVIEW=zoom opens that mod's page
    const char* preview = std::getenv("TALAZ_PREVIEW");
    wanted = preview != nullptr;
    for (mods::Mod& m : mods::all())
        if (preview && std::string(preview) == m.id)
            page = &m;
}

bool key(int key, int action) {
    bool eaten = mods::on_key(key, action, playing() && !wanted);
    // The menu key opens the menu in a world and not over other screens; the
    // game only holds the mouse while nothing is open
    if (action == KEY_PRESS && key == int(config::get("key.menu")) && (wanted || playing())) {
        wanted = !wanted;
        return true;
    }
    if (wanted && action == KEY_PRESS && key == KEY_ESCAPE && page) {
        page = nullptr;
        return true;
    }
    if (wanted && action == KEY_PRESS && key == KEY_ESCAPE) {
        wanted = false;
        return true;
    }
    // the menu keeps key presses from the game but lets releases through, so a key
    // held when the menu opened doesn't stay held in the game after it closes
    return (wanted && action != KEY_RELEASE) || eaten;
}

bool mouse_button(double x, double y, int button, int action) {
    if (!wanted)
        mods::on_mouse_button(button, action);
    // the title screen's Mods button has no game button under it, so the mod takes its clicks
    if (!wanted && on_title && button == 1 && title::owns(ImVec2(float(x), window_y(y)), ImVec2(width, height))) {
        if (action == BUTTON_PRESS)
            title_click = true;
        return true;
    }
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
    if (relative && !wanted && playing())
        mods::on_look(x, y);
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
    mods::riptide(surface);
    step_water(dt);
    mods::limit_fps();
    // the HUDs show over a world being played, not over the game's screens; off a
    // world, the frame is looked at for the title screen's mark
    bool hud = (mods::hud_visible() || mods::effects_visible()) && playing();
    if (!wanted && !hud && playing())
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

    on_title = !playing() && title::showing();
    if (!wanted && !hud && !on_title)
        return;

    ImGuiIO& io = ImGui::GetIO();
    io.DisplaySize = size;
    io.DeltaTime = dt;
    io.MouseDrawCursor = relative_mouse && wanted;

    ImGui_ImplOpenGL3_NewFrame();
    ImGui::NewFrame();
    if (playing())
        mods::draw_effects(ImGui::GetBackgroundDrawList());
    if (on_title && !wanted && title::draw(ImGui::GetBackgroundDrawList(), size, ImVec2(cursor_x, cursor_y), title_click.exchange(false), float(ImGui::GetTime())))
        wanted = true;
    if (wanted)
        draw(size, dt);
    else if (playing())
        mods::draw_hud(ImGui::GetForegroundDrawList(), scale * 1.5f, ImGui::GetFont());
    ImGui::Render();
    ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
}

}  // namespace menu
