#include "mods.h"

#include <android/log.h>
#include <dlfcn.h>
#include <link.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <deque>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include "config.h"
#include "gl.h"
#include "theme.h"

namespace mods {
namespace {

// Key codes as the launcher reports them, which follow Windows' virtual keys
constexpr int KEY_W = 'W', KEY_A = 'A', KEY_S = 'S', KEY_D = 'D', KEY_SPACE = ' ';
constexpr int KEY_PRESS = 0, KEY_RELEASE = 2;
constexpr int BUTTON_PRESS = 0;

Mod list[] = {
        {"zoom", "Zoom", "Hold C to zoom in, scroll to zoom further"},
        {"fps", "FPS", "Frames per second on screen"},
        {"cps", "CPS", "Clicks per second"},
        {"keystrokes", "Keystrokes", "Shows the keys you press"},
        {"clock", "Clock", "The real time, so you know when to stop"},
        {"session", "Session", "How long you have been playing"},
        {"fpslimit", "FPS Limit", "Caps the frame rate to keep the computer cool and quiet"},
        {"togglesprint", "Toggle Sprint", "Press Ctrl once to keep sprinting"},
        {"nightvision", "Night Vision", "Brightens the picture so caves and nights are easy to see"},
        {"motionblur", "Motion Blur", "Blends each frame into the next for a smooth trail"},
};

// The mods that draw on screen, stacked down the top-left at first
// keystrokes is the tall one, so it goes last
const char* const HUDS[] = {"fps", "cps", "clock", "session", "keystrokes"};

// --- input the HUDs watch ---------------------------------------------------

using Clock = std::chrono::steady_clock;

std::atomic<bool> keys[5];  // W A S D Space
std::atomic<bool> buttons[2];  // left, right
// clicks arrive with input and are counted in the frame
std::mutex clicks_lock;
std::deque<Clock::time_point> clicks[2];

int key_slot(int key) {
    switch (key) {
    case KEY_W: return 0;
    case KEY_A: return 1;
    case KEY_S: return 2;
    case KEY_D: return 3;
    case KEY_SPACE: return 4;
    default: return -1;
    }
}

size_t cps[2] = {};
int fps = 0;
int frames = 0;
float fps_window = 0;

// --- clock and session ------------------------------------------------------

const Clock::time_point started = Clock::now();

// --- zoom -------------------------------------------------------------------

std::atomic<bool> zoom_held{false};
bool sprinting = false;
float zoom_now = 1;  // the multiplier applied this frame, eased towards the target

std::optional<float> (*try_get_fov)(void*) = nullptr;

// If a game update moves tryGetFOV again the hooked slot is some other
// function, so Zoom only touches values that look like a field of view and
// gives up after a run of ones that don't
std::atomic<int> implausible{0};
constexpr int GIVE_UP_AFTER = 120;

std::optional<float> zoomed_fov(void* camera) {
    std::optional<float> fov = try_get_fov(camera);
    if (implausible >= GIVE_UP_AFTER)
        return fov;
    if (!fov || !(*fov > 1 && *fov < 179)) {
        if (fov && ++implausible == GIVE_UP_AFTER)
            __android_log_print(ANDROID_LOG_ERROR, CLIENT_NAME, "Zoom turned itself off, this game version moved the camera's field of view");
        return fov;
    }
    implausible = 0;
    *fov *= zoom_now;
    return fov;
}

// Night Vision makes the game believe the player has the effect: every place that
// asks "has night vision?" before asking "how much?" hears yes, and "how much" hears
// full. It looks the same as the potion, leaves the HUD alone and does not stack with it
static uint32_t* night_vision_scale = nullptr;
static uint32_t night_vision_original[2];
static std::vector<std::pair<uint32_t*, uint32_t>> night_vision_checks;
static void* (*patch_code)(void*, void*, size_t) = nullptr;

static void set_night_vision(bool on) {
    static bool now = false;
    if (!night_vision_scale || on == now)
        return;
    const uint32_t full[2] = {0x1E2E1000, 0xD65F03C0}; // fmov s0, #1.0; ret
    patch_code(night_vision_scale, const_cast<uint32_t*>(on ? full : night_vision_original), sizeof full);
    for (auto& [at, original] : night_vision_checks) {
        uint32_t yes = 0x52800020; // mov w0, #1
        patch_code(at, on ? &yes : &original, sizeof yes);
    }
    __android_log_print(ANDROID_LOG_INFO, CLIENT_NAME, "Night Vision %s at %p: %08x %08x", on ? "on" : "off", night_vision_scale, night_vision_scale[0], night_vision_scale[1]);
    now = on;
}

struct Segment {
    std::byte* begin;
    size_t size;
    bool exec;
};

// found by the formula's 0.2 pi constant: mov w8, #0xd97c; movk w8, #0x3f20, lsl #16
// ponytail: the start sits 0xd4 before it on 1.21.114.1. Scan back for the prologue if other versions move it
void find_night_vision(const std::vector<Segment>& segs, void* (*patch)(void*, void*, size_t)) {
    for (const Segment& seg : segs) {
        if (!seg.exec)
            continue;
        auto* w = reinterpret_cast<uint32_t*>(seg.begin);
        for (size_t i = 0x40, n = seg.size / 4 - 1; i < n; i++) {
            if (w[i] != 0x529B2F88 || w[i + 1] != 0x72A7E408 || w[i - 0xd4 / 4] != 0xD10143FF)
                continue;
            night_vision_scale = w + i - 0xd4 / 4;
            std::memcpy(night_vision_original, night_vision_scale, sizeof night_vision_original);
            // the callers: the closest bl before each call is the "has the effect?" check
            for (size_t j = 0; j < n; j++) {
                if ((w[j] & 0xfc000000) != 0x94000000 || w + j + (int32_t(w[j] << 6) >> 6) != night_vision_scale)
                    continue;
                for (size_t k = j - 1; k + 8 > j && k > 0; k--)
                    if ((w[k] & 0xfc000000) == 0x94000000) {
                        night_vision_checks.emplace_back(w + k, w[k]);
                        break;
                    }
            }
            __android_log_print(ANDROID_LOG_INFO, CLIENT_NAME, "Night Vision found %zu checks", night_vision_checks.size());
            patch_code = patch;
            return;
        }
    }
    __android_log_print(ANDROID_LOG_ERROR, CLIENT_NAME, "Night Vision couldn't find the game's night vision");
}

std::vector<Segment> segments_of(void* lib) {
    struct Search {
        void* lib;
        std::vector<Segment> out;
    } search{lib, {}};
    dl_iterate_phdr(
            [](dl_phdr_info* info, size_t, void* data) {
                auto* s = static_cast<Search*>(data);
                void* h = dlopen(info->dlpi_name, RTLD_NOLOAD);
                if (h)
                    dlclose(h);
                if (h != s->lib)
                    return 0;
                for (int i = 0; i < info->dlpi_phnum; i++) {
                    const ElfW(Phdr)& ph = info->dlpi_phdr[i];
                    if (ph.p_type == PT_LOAD && (ph.p_flags & PF_R))
                        s->out.push_back({reinterpret_cast<std::byte*>(info->dlpi_addr + ph.p_vaddr), ph.p_memsz,
                                          (ph.p_flags & PF_X) != 0});
                }
                return 1;
            },
            &search);
    return search.out;
}

std::byte* find_string(const std::vector<Segment>& segs, const void* needle, size_t n) {
    for (const Segment& seg : segs)
        if (void* hit = memmem(seg.begin, seg.size, needle, n))
            return static_cast<std::byte*>(hit);
    return nullptr;
}

std::byte* find_pointer(const std::vector<Segment>& segs, const void* value) {
    for (const Segment& seg : segs) {
        auto* words = reinterpret_cast<const void* const*>(seg.begin);
        for (size_t i = 0; i < seg.size / sizeof(void*); i++)
            if (words[i] == value)
                return seg.begin + i * sizeof(void*);
    }
    return nullptr;
}

bool in_code(const std::vector<Segment>& segs, const void* p) {
    for (const Segment& seg : segs)
        if (seg.exec && p >= seg.begin && p < seg.begin + seg.size)
            return true;
    return false;
}

}  // namespace

std::span<Mod> all() { return list; }

bool enabled(const char* id) { return config::on(std::string(id) + ".enabled"); }

void toggle(const char* id) {
    config::set(std::string(id) + ".enabled", enabled(id) ? 0 : 1);
    config::save();
}

void declare() {
    for (const Mod& mod : list)
        config::declare(std::string(mod.id) + ".enabled", 0);
    int i = 0;
    for (const char* hud : HUDS) {
        // fractions of the screen, so a HUD stays put across window sizes
        config::declare(std::string(hud) + ".x", 0.008f);
        config::declare(std::string(hud) + ".y", 0.015f + 0.055f * i++);
    }
    config::declare("fpslimit.cap", 60);
    config::declare("nightvision.strength", 4);
    config::declare("motionblur.amount", 50);
    config::declare("key.zoom", 'C');
    config::declare("key.sprint", 17);
    config::declare("zoom.factor", 4);
}

void limit_fps() {
    static Clock::time_point next;
    auto now = Clock::now();
    if (!enabled("fpslimit")) {
        next = now;
        return;
    }
    next += std::chrono::duration_cast<Clock::duration>(std::chrono::duration<float>(1 / std::max(10.f, config::get("fpslimit.cap"))));
    // a slow frame is not made up for by rushing the next ones
    if (next < now)
        next = now;
    else
        std::this_thread::sleep_until(next);
}

void install_zoom() {
    void* game = dlopen("libminecraftpe.so", 0);
    void* api = dlopen("libmcpelauncher_mod.so", 0);
    if (!game || !api)
        return;
    auto patch = reinterpret_cast<void* (*)(void*, void*, size_t)>(dlsym(api, "mcpelauncher_patch"));
    auto* major = static_cast<int*>(dlsym(api, "mcpelauncher_package_version_major"));
    auto* minor = static_cast<int*>(dlsym(api, "mcpelauncher_package_version_minor"));
    auto* patch_v = static_cast<int*>(dlsym(api, "mcpelauncher_package_version_patch"));
    if (!patch || !major || !minor || !patch_v)
        return;

    // CameraAPI's vtable, found through its RTTI: the type name string, the
    // typeinfo that points at it, then the vtable that points at the typeinfo
    std::vector<Segment> segs = segments_of(game);
    find_night_vision(segs, patch);
    static const char name[] = "\0" "9CameraAPI";
    std::byte* found = find_string(segs, name, sizeof name);
    if (!found)
        return;
    const void* type_name = found + 1;
    std::byte* name_ref = find_pointer(segs, type_name);
    if (!name_ref)
        return;
    const void* typeinfo = name_ref - sizeof(void*);
    std::byte* typeinfo_ref = find_pointer(segs, typeinfo);
    if (!typeinfo_ref)
        return;
    auto** vtable = reinterpret_cast<void**>(typeinfo_ref + sizeof(void*));

    // tryGetFOV moved one slot down in 1.21.130
    bool newer = *major > 1 || *minor > 21 || (*minor == 21 && *patch_v >= 130);
    void** slot = vtable + (newer ? 7 : 6);
    if (!in_code(segs, *slot)) {
        __android_log_print(ANDROID_LOG_ERROR, CLIENT_NAME, "Zoom couldn't find the camera on %d.%d.%d", *major, *minor, *patch_v);
        return;
    }
    try_get_fov = reinterpret_cast<decltype(try_get_fov)>(*slot);
    void* hook = reinterpret_cast<void*>(&zoomed_fov);
    patch(slot, &hook, sizeof hook);
}

bool on_key(int key, int action, bool playing) {
    if (action != KEY_PRESS && action != KEY_RELEASE)
        return false;
    bool down = action == KEY_PRESS;
    // Toggle Sprint: the game keeps seeing Ctrl held, because its release is
    // kept back until the next press
    // ponytail: Ctrl stays held for the game in chat and menus too. Let go there once the mod can send keys itself
    if (key == int(config::get("key.sprint")) && (sprinting || (playing && enabled("togglesprint")))) {
        if (down)
            sprinting = !sprinting;
        return down != sprinting;
    }
    if (int slot = key_slot(key); slot >= 0)
        keys[slot] = down;
    if (key == int(config::get("key.zoom")))
        zoom_held = down && playing && enabled("zoom");
    return false;
}

void on_mouse_button(int button, int action) {
    int index = button - 1;
    if (index < 0 || index > 1)
        return;
    bool down = action == BUTTON_PRESS;
    buttons[index] = down;
    if (down) {
        std::lock_guard lock(clicks_lock);
        clicks[index].push_back(Clock::now());
    }
}

bool on_scroll(double dy, bool playing) {
    if (!zoom_held || !playing)
        return false;
    // each notch zooms by a tenth, in half steps
    float f = config::get("zoom.factor") * (dy > 0 ? 1.1f : 1 / 1.1f);
    config::set("zoom.factor", std::clamp(std::round(f * 2) / 2, 1.5f, 50.f));
    return true;
}

void tick(float dt) {
    frames++;
    fps_window += dt;
    if (fps_window >= 1) {
        fps = int(std::lround(frames / fps_window));
        frames = 0;
        fps_window = 0;
    }
    auto cutoff = Clock::now() - std::chrono::seconds(1);
    {
        std::lock_guard lock(clicks_lock);
        for (int i = 0; i < 2; i++) {
            while (!clicks[i].empty() && clicks[i].front() < cutoff)
                clicks[i].pop_front();
            cps[i] = clicks[i].size();
        }
    }

    set_night_vision(enabled("nightvision"));
    float target = zoom_held ? 1 / config::get("zoom.factor") : 1;
    zoom_now += (target - zoom_now) * (1 - std::exp(-dt * 12));
}

bool effects_visible() {
    return enabled("motionblur");
}

// The effects change the finished picture, so they also touch the game's own
// HUD. All GL calls sit in draw callbacks, where ImGui has saved the game's state
// ponytail: effects over the whole screen. Draw them before the game's HUD once the mod hooks the renderer
void draw_effects(ImDrawList* dl) {
    static GLuint last = 0;
    static bool have_last = false;
    ImVec2 size = ImGui::GetIO().DisplaySize;
    if (!enabled("motionblur")) {
        have_last = false;
        return;
    }
    if (have_last) {
        int alpha = int(255 * std::clamp(config::get("motionblur.amount") / 100, 0.f, 0.9f));
        // a copied frame is stored bottom up
        dl->AddImage(ImTextureID(intptr_t(last)), ImVec2(0, 0), size, ImVec2(0, 1), ImVec2(1, 0), IM_COL32(255, 255, 255, alpha));
    }
    dl->AddCallback([](const ImDrawList*, const ImDrawCmd*) {
        ImVec2 size = ImGui::GetIO().DisplaySize;
        bool fresh = !last;
        if (fresh)
            glGenTextures(1, &last);
        glBindTexture(GL_TEXTURE_2D, last);
        if (fresh) {
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        }
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 0, 0, GLsizei(size.x), GLsizei(size.y), 0);
        have_last = true;
    }, nullptr);
    dl->AddCallback(ImDrawCallback_ResetRenderState, nullptr);
}

bool hud_visible() {
    return std::any_of(std::begin(HUDS), std::end(HUDS), [](const char* hud) { return enabled(hud); });
}

void draw_hud(ImDrawList* dl, float u, ImFont* font) {
    using namespace theme;
    ImVec2 display = ImGui::GetIO().DisplaySize;
    auto at = [&](const char* id) {
        return ImVec2(std::round(config::get(std::string(id) + ".x") * display.x),
                      std::round(config::get(std::string(id) + ".y") * display.y));
    };
    auto label = [&](const char* id, const char* str) {
        ImVec2 a = at(id);
        float size = 14 * u, px = 10 * u, py = 6 * u;
        ImVec2 box = font->CalcTextSizeA(size, FLT_MAX, 0, str);
        dl->AddRectFilled(a, ImVec2(a.x + box.x + 2 * px, a.y + box.y + 2 * py), PANEL, 8 * u);
        dl->AddText(font, size, ImVec2(std::round(a.x + px), std::round(a.y + py)), TEXT, str);
    };

    char buf[64];
    if (enabled("fps")) {
        std::snprintf(buf, sizeof buf, "%d FPS", fps);
        label("fps", buf);
    }
    if (enabled("cps")) {
        std::snprintf(buf, sizeof buf, "%zu | %zu CPS", cps[0], cps[1]);
        label("cps", buf);
    }
    if (enabled("clock")) {
        time_t now = time(nullptr);
        tm parts{};
        localtime_r(&now, &parts);
        std::snprintf(buf, sizeof buf, "%02d:%02d", parts.tm_hour, parts.tm_min);
        label("clock", buf);
    }
    if (enabled("session")) {
        long s = std::chrono::duration_cast<std::chrono::seconds>(Clock::now() - started).count();
        if (s >= 3600)
            std::snprintf(buf, sizeof buf, "%ld:%02ld:%02ld", s / 3600, s / 60 % 60, s % 60);
        else
            std::snprintf(buf, sizeof buf, "%ld:%02ld", s / 60, s % 60);
        label("session", buf);
    }
    if (enabled("keystrokes")) {
        ImVec2 o = at("keystrokes");
        float k = 34 * u, gap = 4 * u, size = 12.5f * u;
        // a pressed key fills with the accent, like a tile that is on
        auto key = [&](float x, float y, float w, float h, bool down, const char* str) {
            ImVec2 a(o.x + x, o.y + y), b(a.x + w, a.y + h);
            dl->AddRectFilled(a, b, down ? ACCENT : PANEL, 8 * u);
            ImVec2 box = font->CalcTextSizeA(size, FLT_MAX, 0, str);
            dl->AddText(font, size, ImVec2(std::round((a.x + b.x - box.x) / 2), std::round((a.y + b.y - box.y) / 2)),
                        down ? DEEP : TEXT, str);
        };
        float row = k + gap, wide = (3 * k + gap) / 2;
        key(row, 0, k, k, keys[0], "W");
        key(0, row, k, k, keys[1], "A");
        key(row, row, k, k, keys[2], "S");
        key(2 * row, row, k, k, keys[3], "D");
        key(0, 2 * row, wide, k, buttons[0], "LMB");
        key(wide + gap, 2 * row, wide, k, buttons[1], "RMB");
        key(0, 3 * row, 3 * k + 2 * gap, k / 2, keys[4], "");
    }
}

}  // namespace mods
