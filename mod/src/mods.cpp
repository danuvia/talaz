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
        {"motionblur", "Motion Blur", "Smears the world the way the camera turns"},
        {"camerasway", "Camera Sway", "Tilts the camera as you strafe and kicks it up on a jump"},
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

float last_fov = 70;  // the field of view the world was last drawn with, for Camera Sway

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
    last_fov = *fov * zoom_now;
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

// The game builds a texture each frame from an image stored inside it that it
// can't read under mcpelauncher, and logs a failure every time. Point it at an
// empty PNG instead: nothing it showed goes away, and the retries stop
// ponytail: the offsets are 1.21.114.1's, checked against the instructions before patching. Find them by pattern for other versions
static const unsigned char blank_png[] = {
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0b, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0x7a, 0x5e, 0xab, 0x3f, 0x00,
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82};

void fix_unreadable_image(std::byte* base, void* (*patch)(void*, void*, size_t)) {
    auto* at = reinterpret_cast<uint32_t*>(base + 0x64a1978);
    // adrp x1; mov w2, #0x530c; add x1, x1; ...; movk w2, #1, lsl #16
    if (at[0] != 0x90050d21 || at[1] != 0x528a6182 || at[2] != 0x913d0021 || at[5] != 0x72a00022)
        return;
    auto pc = reinterpret_cast<uintptr_t>(at), png = reinterpret_cast<uintptr_t>(blank_png);
    int64_t pages = int64_t(png >> 12) - int64_t(pc >> 12);
    if (pages >= (1 << 20) || pages < -(1 << 20))
        return;
    uint32_t adrp = 0x90000001 | uint32_t(pages & 3) << 29 | uint32_t((pages >> 2) & 0x7ffff) << 5;
    uint32_t add = 0x91000021 | uint32_t(png & 0xfff) << 10;
    uint32_t len = sizeof blank_png;
    uint32_t code[3] = {adrp, 0x52800002 | (len & 0xffff) << 5, add};
    uint32_t high = 0x72a00002 | (len >> 16) << 5;
    patch(at, code, sizeof code);
    patch(at + 5, &high, sizeof high);
    __android_log_print(ANDROID_LOG_INFO, CLIENT_NAME, "Pointed the unreadable built-in image at a blank one");
}

// Motion blur sits inside the game's frame: the game draws the world straight to
// the screen, then clears only the depth before the hand and the interface. At
// that clear the last frame is laid over the world and the result kept for the
// next one, so the world trails and the hand and the HUDs stay sharp
// How far the camera turned since the last frame, in screen pixels, from the
// game's own mouse motion while it holds the mouse
std::atomic<float> look_x{0}, look_y{0};
float look_vx = 0, look_vy = 0;  // the turn this frame, for the camera blur

void (*real_clear)(GLbitfield) = nullptr;
std::atomic<bool> blurred_this_frame{false};
float blur_keep = 0;  // how much of the last frame stays, set each frame from the amount and frame time

// Like PolyBlur's Unity mode: the world is smeared along the way the camera turns,
// sharp when it holds still
// ponytail: the turn comes from mouse motion, so turning with a controller isn't blurred. Read the camera's angles once the mod finds them
void camera_blur(GLint vp[4]) {
    static GLuint program = 0, vao = 0, frame = 0;
    // ponytail: mouse counts to screen pixels is a guess (twice), tune once the camera angles are read
    float amount = 2 * std::clamp(config::get("motionblur.amount") / 100, 0.f, 1.f);
    float dx = look_vx * amount / vp[2], dy = -look_vy * amount / vp[3];
    if (std::abs(dx * vp[2]) + std::abs(dy * vp[3]) < 0.5f)
        return;
    if (!program) {
        const char* vs = "#version 300 es\nout vec2 uv;\n"
                         "void main() { vec2 p = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);"
                         " uv = p * 0.5 + 0.5; gl_Position = vec4(p, 0, 1); }";
        const char* fs = "#version 300 es\nprecision mediump float; uniform sampler2D frame; uniform vec2 step; uniform int samples; in vec2 uv; out vec4 color;\n"
                         "void main() { vec3 sum = vec3(0); for (int i = 0; i < samples; i++) {"
                         " float t = float(i) / float(samples - 1) - 0.5; sum += texture(frame, clamp(uv + step * t, 0.0, 1.0)).rgb; }"
                         " color = vec4(sum / float(samples), 1); }";
        program = glCreateProgram();
        for (auto [type, src] : {std::pair{GL_VERTEX_SHADER, vs}, std::pair{GL_FRAGMENT_SHADER, fs}}) {
            GLuint shader = glCreateShader(type);
            glShaderSource(shader, 1, &src, nullptr);
            glCompileShader(shader);
            glAttachShader(program, shader);
            glDeleteShader(shader);
        }
        glLinkProgram(program);
        glGenVertexArrays(1, &vao);
        glGenTextures(1, &frame);
        glBindTexture(GL_TEXTURE_2D, frame);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    }
    glBindTexture(GL_TEXTURE_2D, frame);
    glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, vp[0], vp[1], vp[2], vp[3], 0);
    glDisable(GL_BLEND);
    glUseProgram(program);
    glUniform1i(glGetUniformLocation(program, "frame"), 0);
    glUniform2f(glGetUniformLocation(program, "step"), dx, dy);
    glUniform1i(glGetUniformLocation(program, "samples"), std::clamp(int(config::get("motionblur.samples")), 2, 64));
    glBindVertexArray(vao);
    glDrawArrays(GL_TRIANGLES, 0, 3);
}

// Camera Sway, like OneClient's Camera Overhaul: the world tilts as the player
// strafes and kicks up on a jump, then springs back. The finished world is turned
// and shifted a few degrees, which at these angles looks like turning the camera,
// and drawn a little larger so its edges never show. The hand and HUD stay put
// ponytail: driven by the keys, not the player's real motion, so a jump in the air or water kicks too. Read the player's velocity once the mod finds it
std::atomic<bool> jump_pressed{false};
float sway_roll = 0, sway_roll_v = 0, sway_pitch = 0, sway_pitch_v = 0;  // degrees

bool sway_active() {
    return enabled("camerasway") && (std::abs(sway_roll) > 0.01f || std::abs(sway_pitch) > 0.01f);
}

void sway_tick(float dt, bool jumped) {
    float roll_to = enabled("camerasway") ? (float(keys[3]) - float(keys[1])) * config::get("camerasway.tilt") : 0;
    if (jumped && enabled("camerasway"))
        sway_pitch_v += config::get("camerasway.jump") * 14;
    // springs: stiff enough to settle in about half a second, damped so they don't wobble
    sway_roll_v += ((roll_to - sway_roll) * 40 - sway_roll_v * 11) * dt;
    sway_roll += sway_roll_v * dt;
    sway_pitch_v += (-sway_pitch * 60 - sway_pitch_v * 10) * dt;
    sway_pitch += sway_pitch_v * dt;
}

void sway_world(GLint vp[4]) {
    static GLuint program = 0, vao = 0, frame = 0;
    if (!program) {
        const char* vs = "#version 300 es\nout vec2 uv;\n"
                         "void main() { vec2 p = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);"
                         " uv = p * 0.5 + 0.5; gl_Position = vec4(p, 0, 1); }";
        const char* fs = "#version 300 es\nprecision highp float; uniform sampler2D frame; uniform vec2 turn; uniform vec2 shift;"
                         " uniform float aspect; uniform float grow; in vec2 uv; out vec4 color;\n"
                         "void main() { vec2 p = (uv - 0.5) * vec2(aspect, 1.0) / grow;"
                         " p = vec2(p.x * turn.x - p.y * turn.y, p.x * turn.y + p.y * turn.x);"
                         " color = vec4(texture(frame, p / vec2(aspect, 1.0) + 0.5 + shift).rgb, 1); }";
        program = glCreateProgram();
        for (auto [type, src] : {std::pair{GL_VERTEX_SHADER, vs}, std::pair{GL_FRAGMENT_SHADER, fs}}) {
            GLuint shader = glCreateShader(type);
            glShaderSource(shader, 1, &src, nullptr);
            glCompileShader(shader);
            glAttachShader(program, shader);
            glDeleteShader(shader);
        }
        glLinkProgram(program);
        glGenVertexArrays(1, &vao);
        glGenTextures(1, &frame);
        glBindTexture(GL_TEXTURE_2D, frame);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    }
    float aspect = float(vp[2]) / float(vp[3]), roll = sway_roll * 3.14159265f / 180;
    // a pitch of a degrees moves the picture by tan(a) over tan(fov / 2) of half its height
    float shift = -0.5f * std::tan(sway_pitch * 3.14159265f / 180) / std::tan(last_fov * 3.14159265f / 360);
    float grow = std::abs(std::cos(roll)) + std::abs(std::sin(roll)) * aspect + 2 * std::abs(shift);
    glBindTexture(GL_TEXTURE_2D, frame);
    glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, vp[0], vp[1], vp[2], vp[3], 0);
    glDisable(GL_BLEND);
    glUseProgram(program);
    glUniform1i(glGetUniformLocation(program, "frame"), 0);
    glUniform2f(glGetUniformLocation(program, "turn"), std::cos(roll), std::sin(roll));
    glUniform2f(glGetUniformLocation(program, "shift"), 0, shift);
    glUniform1f(glGetUniformLocation(program, "aspect"), aspect);
    glUniform1f(glGetUniformLocation(program, "grow"), grow);
    glBindVertexArray(vao);
    glDrawArrays(GL_TRIANGLES, 0, 3);
}

void blur_world() {
    static GLuint program = 0, vao = 0, last = 0;
    static GLint last_w = 0, last_h = 0;
    GLint vp[4];
    glGetIntegerv(GL_VIEWPORT, vp);
    if (vp[2] <= 0 || vp[3] <= 0)
        return;
    // the game's state, put back afterwards
    GLint prog, va, tex, active, src_rgb, dst_rgb, src_a, dst_a, eq_rgb, eq_a;
    glGetIntegerv(GL_CURRENT_PROGRAM, &prog);
    glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &va);
    glGetIntegerv(GL_ACTIVE_TEXTURE, &active);
    glActiveTexture(GL_TEXTURE0);
    glGetIntegerv(GL_TEXTURE_BINDING_2D, &tex);
    glGetIntegerv(GL_BLEND_SRC_RGB, &src_rgb);
    glGetIntegerv(GL_BLEND_DST_RGB, &dst_rgb);
    glGetIntegerv(GL_BLEND_SRC_ALPHA, &src_a);
    glGetIntegerv(GL_BLEND_DST_ALPHA, &dst_a);
    glGetIntegerv(GL_BLEND_EQUATION_RGB, &eq_rgb);
    glGetIntegerv(GL_BLEND_EQUATION_ALPHA, &eq_a);
    GLboolean mask[4];
    glGetBooleanv(GL_COLOR_WRITEMASK, mask);
    bool blend = glIsEnabled(GL_BLEND), depth = glIsEnabled(GL_DEPTH_TEST), cull = glIsEnabled(GL_CULL_FACE),
         scissor = glIsEnabled(GL_SCISSOR_TEST), stencil = glIsEnabled(GL_STENCIL_TEST);
    if (sway_active()) {
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
        sway_world(vp);
    }
    if (blur_keep > 0) {

    if (!program) {
        // one triangle over the whole screen, made from the vertex number alone
        const char* vs = "#version 300 es\nout vec2 uv;\n"
                         "void main() { vec2 p = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);"
                         " uv = p * 0.5 + 0.5; gl_Position = vec4(p, 0, 1); }";
        const char* fs = "#version 300 es\nprecision mediump float; uniform sampler2D last; uniform float keep; in vec2 uv; out vec4 color;\n"
                         "void main() { color = vec4(texture(last, uv).rgb, keep); }";
        program = glCreateProgram();
        for (auto [type, src] : {std::pair{GL_VERTEX_SHADER, vs}, std::pair{GL_FRAGMENT_SHADER, fs}}) {
            GLuint shader = glCreateShader(type);
            glShaderSource(shader, 1, &src, nullptr);
            glCompileShader(shader);
            glAttachShader(program, shader);
            glDeleteShader(shader);
        }
        glLinkProgram(program);
        glGenVertexArrays(1, &vao);
        glGenTextures(1, &last);
        glBindTexture(GL_TEXTURE_2D, last);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    }
    if (config::get("motionblur.camera") != 0) {
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
        camera_blur(vp);
        last_w = 0;  // the trail starts over if it is switched back
    } else {
    glBindTexture(GL_TEXTURE_2D, last);
    // a new size starts the trail over
    if (vp[2] == last_w && vp[3] == last_h) {
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_CULL_FACE);
        glDisable(GL_SCISSOR_TEST);
        glDisable(GL_STENCIL_TEST);
        glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
        glEnable(GL_BLEND);
        glBlendEquation(GL_FUNC_ADD);
        glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
        glUseProgram(program);
        glUniform1i(glGetUniformLocation(program, "last"), 0);
        glUniform1f(glGetUniformLocation(program, "keep"), blur_keep);
        glBindVertexArray(vao);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
    glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, vp[0], vp[1], vp[2], vp[3], 0);
    last_w = vp[2];
    last_h = vp[3];
    }
    }

    glUseProgram(prog);
    glBindVertexArray(va);
    glBindTexture(GL_TEXTURE_2D, tex);
    glActiveTexture(active);
    glBlendFuncSeparate(src_rgb, dst_rgb, src_a, dst_a);
    glBlendEquationSeparate(eq_rgb, eq_a);
    glColorMask(mask[0], mask[1], mask[2], mask[3]);
    (blend ? glEnable : glDisable)(GL_BLEND);
    (depth ? glEnable : glDisable)(GL_DEPTH_TEST);
    (cull ? glEnable : glDisable)(GL_CULL_FACE);
    (scissor ? glEnable : glDisable)(GL_SCISSOR_TEST);
    (stencil ? glEnable : glDisable)(GL_STENCIL_TEST);
}

void clear_hook(GLbitfield bits) {
    // only the screen's own depth clear counts: the paper doll and other pictures
    // are drawn in framebuffers of their own and clear there first
    if (bits == GL_DEPTH_BUFFER_BIT && (blur_keep > 0 || sway_active()) && tc_glDrawArrays && !blurred_this_frame) {
        GLint fb = 0;
        glGetIntegerv(GL_FRAMEBUFFER_BINDING, &fb);
        if (fb == 0) {
            blurred_this_frame = true;
            blur_world();
        }
    }
    real_clear(bits);
}

// Points one of the game's imported functions at a replacement by rewriting
// its slot in the game's table of imports. Returns what was there
void* hook_import(const char* name, void* replacement, void* (*patch)(void*, void*, size_t)) {
    struct Search {
        const char* name;
        void* slot = nullptr;
    } search{name};
    void* game = dlopen("libminecraftpe.so", RTLD_NOLOAD);
    dl_iterate_phdr(
            [](dl_phdr_info* info, size_t, void* data) {
                auto* s = static_cast<Search*>(data);
                if (!info->dlpi_name || !std::strstr(info->dlpi_name, "libminecraftpe.so"))
                    return 0;
                const ElfW(Sym)* symtab = nullptr;
                const char* strtab = nullptr;
                const ElfW(Rela)* plt = nullptr;
                size_t pltsize = 0;
                for (int i = 0; i < info->dlpi_phnum; i++) {
                    if (info->dlpi_phdr[i].p_type != PT_DYNAMIC)
                        continue;
                    for (auto* d = reinterpret_cast<const ElfW(Dyn)*>(info->dlpi_addr + info->dlpi_phdr[i].p_vaddr); d->d_tag != DT_NULL; d++) {
                        if (d->d_tag == DT_SYMTAB)
                            symtab = reinterpret_cast<const ElfW(Sym)*>(info->dlpi_addr + d->d_un.d_ptr);
                        else if (d->d_tag == DT_STRTAB)
                            strtab = reinterpret_cast<const char*>(info->dlpi_addr + d->d_un.d_ptr);
                        else if (d->d_tag == DT_JMPREL)
                            plt = reinterpret_cast<const ElfW(Rela)*>(info->dlpi_addr + d->d_un.d_ptr);
                        else if (d->d_tag == DT_PLTRELSZ)
                            pltsize = d->d_un.d_val;
                    }
                }
                if (!symtab || !strtab || !plt)
                    return 1;
                for (size_t i = 0; i < pltsize / sizeof(ElfW(Rela)); i++)
                    if (std::strcmp(strtab + symtab[ELF64_R_SYM(plt[i].r_info)].st_name, s->name) == 0)
                        s->slot = reinterpret_cast<void*>(info->dlpi_addr + plt[i].r_offset);
                return 1;
            },
            &search);
    (void)game;
    if (!search.slot)
        return nullptr;
    void* original = *static_cast<void**>(search.slot);
    patch(search.slot, &replacement, sizeof replacement);
    return original;
}

void install_blur(void* (*patch)(void*, void*, size_t)) {
    real_clear = reinterpret_cast<decltype(real_clear)>(hook_import("glClear", (void*)clear_hook, patch));
    if (!real_clear)
        __android_log_print(ANDROID_LOG_ERROR, CLIENT_NAME, "Motion Blur couldn't find the game's glClear");
}

// The game's "hide paper doll" setting, caught as the game registers it: the
// Options object's registerOption is called once per setting, and the one called
// from the paper doll's line is this setting. Its value is the byte at 0x10
// ponytail: the offsets are 1.21.114.1's, checked before patching. Unknown versions keep the HUDs beside the paper doll's spot
const unsigned char* hide_paper_doll = nullptr;
void* (*real_register_option)(void*, void*) = nullptr;
uintptr_t paper_doll_call = 0;

void* register_option_hook(void* options, void* option) {
    void* registered = real_register_option(options, option);
    if (reinterpret_cast<uintptr_t>(__builtin_return_address(0)) == paper_doll_call)
        hide_paper_doll = static_cast<const unsigned char*>(registered) + 0x10,
        __android_log_print(ANDROID_LOG_INFO, CLIENT_NAME, "Found the paper doll setting");
    return registered;
}

void watch_paper_doll(std::byte* base, void* (*patch)(void*, void*, size_t)) {
    auto** slot = reinterpret_cast<void**>(base + 0x1003e290 + 0x80);
    if (*slot != base + 0x6c6a868)
        return;
    real_register_option = reinterpret_cast<decltype(real_register_option)>(*slot);
    paper_doll_call = reinterpret_cast<uintptr_t>(base + 0x6c7f2ac);
    void* hook = reinterpret_cast<void*>(&register_option_hook);
    patch(slot, &hook, sizeof hook);
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

// Every mod's settings, shown on its own page in the menu: a slider from min to
// max, or a switch when there is no step
const Setting SETTINGS[] = {
        {"zoom", "Zoom", "zoom.factor", "\u00d7", 1.5f, 10, 0.5f},
        {"zoom", "Smooth zoom", "zoom.smooth"},
        {"fpslimit", "Frame cap", "fpslimit.cap", " fps", 20, 240, 5},
        {"motionblur", "Follow the camera", "motionblur.camera"},
        {"motionblur", "Amount", "motionblur.amount", "%", 10, 90, 5},
        {"motionblur", "Samples", "motionblur.samples", "", 4, 32, 2},
        {"camerasway", "Strafe tilt", "camerasway.tilt", "\u00b0", 0, 6, 0.5f},
        {"camerasway", "Jump kick", "camerasway.jump", "\u00b0", 0, 6, 0.5f},
        {"clock", "24-hour clock", "clock.24h"},
        {"keystrokes", "Mouse buttons", "keystrokes.mouse"},
        {"keystrokes", "Space bar", "keystrokes.space"},
};

std::vector<const Setting*> settings_of(const char* id) {
    std::vector<const Setting*> out;
    for (const Setting& s : SETTINGS)
        if (std::strcmp(s.mod, id) == 0)
            out.push_back(&s);
    return out;
}

std::string setting_text(const Setting& s, float value) {
    if (s.is_switch())
        return value != 0 ? "On" : "Off";
    char buf[32];
    std::snprintf(buf, sizeof buf, "%g%s", value, s.unit);
    return buf;
}

std::string option_text(const char* id) {
    auto list = settings_of(id);
    return list.empty() || list[0]->is_switch() ? std::string() : setting_text(*list[0], config::get(list[0]->key));
}

void set_setting(const Setting& s, float value, bool save) {
    if (!s.is_switch())
        value = std::clamp(s.min + std::round((value - s.min) / s.step) * s.step, s.min, s.max);
    config::set(s.key, value);
    if (save)
        config::save();
}

void toggle(const char* id) {
    config::set(std::string(id) + ".enabled", enabled(id) ? 0 : 1);
    config::save();
}

void declare() {
    for (const Mod& mod : list)
        config::declare(std::string(mod.id) + ".enabled", 0);
    config::declare("fpslimit.cap", 60);
    config::declare("motionblur.amount", 50);
    config::declare("key.zoom", 'C');
    config::declare("key.sprint", 17);
    config::declare("zoom.factor", 4);
    config::declare("zoom.smooth", 1);
    config::declare("motionblur.camera", 1);
    config::declare("camerasway.tilt", 2);
    config::declare("camerasway.jump", 2);
    config::declare("motionblur.samples", 12);
    config::declare("clock.24h", 1);
    config::declare("keystrokes.mouse", 1);
    config::declare("keystrokes.space", 1);
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
    install_blur(patch);
    if (!segs.empty() && segs.front().exec)
        fix_unreadable_image(segs.front().begin, patch), watch_paper_doll(segs.front().begin, patch);
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
    if (int slot = key_slot(key); slot >= 0) {
        if (slot == 4 && down && !keys[4] && playing)
            jump_pressed = true;
        keys[slot] = down;
    }
    if (key == int(config::get("key.zoom")))
        zoom_held = down && playing && enabled("zoom");
    return false;
}

void on_look(double dx, double dy) {
    look_x = look_x + float(dx);
    look_y = look_y + float(dy);
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
    // a new frame may blur again; the last frame keeps amount of itself per 60th of a
    // second, like the Java mods, so the trail is as long at 200 fps as at 60
    blurred_this_frame = false;
    sway_tick(dt, jump_pressed.exchange(false));
    // the turn since the last frame, scaled to a 60th of a second so the blur is as
    // long at any frame rate
    look_vx = look_x.exchange(0) / std::max(dt * 60, 0.1f);
    look_vy = look_y.exchange(0) / std::max(dt * 60, 0.1f);
    blur_keep = enabled("motionblur") ? std::pow(std::clamp(config::get("motionblur.amount") / 100, 0.f, 0.95f), dt * 60) : 0;
    float target = zoom_held ? 1 / config::get("zoom.factor") : 1;
    zoom_now = config::get("zoom.smooth") != 0 ? zoom_now + (target - zoom_now) * (1 - std::exp(-dt * 12)) : target;
}

// Motion Blur draws nothing itself, but it needs the menu's GL functions loaded
bool effects_visible() {
    return enabled("motionblur") || enabled("camerasway");
}

// Nothing drawn over the finished picture any more; Motion Blur sits in the game's frame
void draw_effects(ImDrawList*) {}

bool hud_visible() {
    return std::any_of(std::begin(HUDS), std::end(HUDS), [](const char* hud) { return enabled(hud); });
}

bool paper_doll() {
    return !hide_paper_doll || *hide_paper_doll == 0;
}

// The HUDs stack down from the top left in a fixed order, each below the one
// before, so they never overlap. The game's paper doll only shows while the player
// moves, so when it is turned on the HUDs keep beside its spot rather than below
// ponytail: the paper doll's width is guessed from the screen height. Ask the game for its HUD layout once the mod can
void draw_hud(ImDrawList* dl, float u, ImFont* font) {
    using namespace theme;
    ImVec2 display = ImGui::GetIO().DisplaySize;
    float x = std::round(paper_doll() ? display.y * 0.12f : 12 * u), y = std::round(12 * u), gap = 6 * u;
    auto label = [&](const char* str) {
        float size = 14 * u, px = 10 * u, py = 6 * u;
        ImVec2 box = font->CalcTextSizeA(size, FLT_MAX, 0, str);
        dl->AddRectFilled(ImVec2(x, y), ImVec2(x + box.x + 2 * px, y + box.y + 2 * py), PANEL, 8 * u);
        dl->AddText(font, size, ImVec2(std::round(x + px), std::round(y + py)), TEXT, str);
        y = std::round(y + box.y + 2 * py + gap);
    };

    char buf[64];
    if (enabled("fps")) {
        std::snprintf(buf, sizeof buf, "%d FPS", fps);
        label(buf);
    }
    if (enabled("cps")) {
        std::snprintf(buf, sizeof buf, "%zu | %zu CPS", cps[0], cps[1]);
        label(buf);
    }
    if (enabled("clock")) {
        time_t now = time(nullptr);
        tm parts{};
        localtime_r(&now, &parts);
        if (config::get("clock.24h") != 0)
            std::snprintf(buf, sizeof buf, "%02d:%02d", parts.tm_hour, parts.tm_min);
        else
            std::snprintf(buf, sizeof buf, "%d:%02d %s", (parts.tm_hour + 11) % 12 + 1, parts.tm_min, parts.tm_hour < 12 ? "AM" : "PM");
        label(buf);
    }
    if (enabled("session")) {
        long s = std::chrono::duration_cast<std::chrono::seconds>(Clock::now() - started).count();
        if (s >= 3600)
            std::snprintf(buf, sizeof buf, "%ld:%02ld:%02ld", s / 3600, s / 60 % 60, s % 60);
        else
            std::snprintf(buf, sizeof buf, "%ld:%02ld", s / 60, s % 60);
        label(buf);
    }
    if (enabled("keystrokes")) {
        ImVec2 o(x, y);
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
        float next = 2 * row;
        if (config::get("keystrokes.mouse") != 0) {
            key(0, next, wide, k, buttons[0], "LMB");
            key(wide + gap, next, wide, k, buttons[1], "RMB");
            next += row;
        }
        if (config::get("keystrokes.space") != 0)
            key(0, next, 3 * k + 2 * gap, k / 2, keys[4], "");
    }
}

}  // namespace mods
