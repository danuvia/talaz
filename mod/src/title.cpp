#include "title.h"

#include <algorithm>
#include <cmath>
#include <cstring>

#include "gl.h"
#include "theme.h"

namespace title {
namespace {

using namespace theme;
constexpr float PI = 3.14159265f;

enum Kind { PRIMARY, PLAIN, ICON };

// Fractions of the screen; the launcher's pack puts the game's buttons on the same
// spots (TITLE_BUTTONS in app/src-tauri/src/main.rs)
struct Button {
    const char* label;
    const char* icon;
    float x, y, w, h;
    Kind kind;
    bool mods = false;  // no game button under it: the mod takes the click
};

const Button buttons[] = {
        {"Play", "play", 0.07f, 0.47f, 0.27f, 0.08f, PRIMARY},
        {"Servers", "servers", 0.07f, 0.57f, 0.27f, 0.065f, PLAIN},
        {"Realms", "realms", 0.07f, 0.65f, 0.27f, 0.065f, PLAIN},
        {"Settings", "settings", 0.07f, 0.73f, 0.13f, 0.065f, PLAIN},
        {"Mods", "waves", 0.21f, 0.73f, 0.13f, 0.065f, PLAIN, true},
        {"Profile", "profile", 0.74f, 0.88f, 0.115f, 0.065f, PLAIN},
        {"Wardrobe", "wardrobe", 0.865f, 0.88f, 0.115f, 0.065f, PLAIN},
        // the top row: the switch to the game's own screen first, then inbox and quit
        {nullptr, "cube", 0.865f, 0.03f, 0.035f, 0.06f, ICON},
        {nullptr, "bell", 0.905f, 0.03f, 0.035f, 0.06f, ICON},
        {nullptr, "close", 0.945f, 0.03f, 0.035f, 0.06f, ICON},
};

// the screen's colour next to each mark, to paint over it
unsigned char beside[4] = {}, beside_middle[4] = {};
// the game's own title screen is up, behind the pack's switch
bool vanilla = false;

ImVec2 corner(const Button& b, ImVec2 d) { return ImVec2(std::round(b.x * d.x), std::round(b.y * d.y)); }
ImVec2 far(const Button& b, ImVec2 d) { return ImVec2(std::round((b.x + b.w) * d.x), std::round((b.y + b.h) * d.y)); }

bool inside(const Button& b, ImVec2 p, ImVec2 d) {
    ImVec2 a = corner(b, d), z = far(b, d);
    return p.x >= a.x && p.x < z.x && p.y >= a.y && p.y < z.y;
}

// Small line icons in the square at a, size s, drawn the way Tide's launcher draws its own
void icon(ImDrawList* dl, const char* name, ImVec2 a, float s, ImU32 c) {
    float w = std::max(1.5f, s * 0.11f);
    auto p = [&](float x, float y) { return ImVec2(a.x + x * s, a.y + y * s); };
    if (!std::strcmp(name, "play")) {
        dl->AddTriangleFilled(p(0.28f, 0.16f), p(0.28f, 0.84f), p(0.84f, 0.5f), c);
    } else if (!std::strcmp(name, "servers")) {
        dl->AddRect(p(0.12f, 0.16f), p(0.88f, 0.44f), c, s * 0.08f, 0, w);
        dl->AddRect(p(0.12f, 0.56f), p(0.88f, 0.84f), c, s * 0.08f, 0, w);
        dl->AddCircleFilled(p(0.28f, 0.3f), s * 0.05f, c);
        dl->AddCircleFilled(p(0.28f, 0.7f), s * 0.05f, c);
    } else if (!std::strcmp(name, "realms")) {
        dl->AddCircle(p(0.5f, 0.5f), s * 0.36f, c, 24, w);
        dl->AddLine(p(0.14f, 0.5f), p(0.86f, 0.5f), c, w);
        dl->AddBezierQuadratic(p(0.5f, 0.14f), p(0.24f, 0.5f), p(0.5f, 0.86f), c, w);
        dl->AddBezierQuadratic(p(0.5f, 0.14f), p(0.76f, 0.5f), p(0.5f, 0.86f), c, w);
    } else if (!std::strcmp(name, "settings")) {
        for (int i = 0; i < 8; i++) {
            float t = i * PI / 4;
            dl->AddLine(p(0.5f + 0.24f * std::cos(t), 0.5f + 0.24f * std::sin(t)), p(0.5f + 0.4f * std::cos(t), 0.5f + 0.4f * std::sin(t)), c, w * 1.6f);
        }
        dl->AddCircle(p(0.5f, 0.5f), s * 0.24f, c, 20, w);
        dl->AddCircle(p(0.5f, 0.5f), s * 0.09f, c, 12, w);
    } else if (!std::strcmp(name, "waves")) {
        for (float y : {0.36f, 0.64f}) {
            for (int i = 0; i <= 16; i++)
                dl->PathLineTo(p(0.1f + 0.8f * i / 16, y - 0.1f * std::sin(i * PI / 8)));
            dl->PathStroke(c, 0, w);
        }
    } else if (!std::strcmp(name, "profile")) {
        dl->AddCircle(p(0.5f, 0.34f), s * 0.17f, c, 16, w);
        dl->PathArcTo(p(0.5f, 0.92f), s * 0.34f, PI * 1.1f, PI * 1.9f, 16);
        dl->PathStroke(c, 0, w);
    } else if (!std::strcmp(name, "wardrobe")) {
        ImVec2 shirt[] = {p(0.36f, 0.16f), p(0.12f, 0.3f), p(0.22f, 0.48f), p(0.3f, 0.43f), p(0.3f, 0.86f),
                          p(0.7f, 0.86f), p(0.7f, 0.43f), p(0.78f, 0.48f), p(0.88f, 0.3f), p(0.64f, 0.16f)};
        dl->AddPolyline(shirt, 10, c, ImDrawFlags_Closed, w);
    } else if (!std::strcmp(name, "cube")) {
        ImVec2 top[] = {p(0.5f, 0.12f), p(0.86f, 0.3f), p(0.5f, 0.48f), p(0.14f, 0.3f)};
        dl->AddPolyline(top, 4, c, ImDrawFlags_Closed, w);
        dl->AddLine(p(0.14f, 0.3f), p(0.14f, 0.7f), c, w);
        dl->AddLine(p(0.86f, 0.3f), p(0.86f, 0.7f), c, w);
        dl->AddLine(p(0.5f, 0.48f), p(0.5f, 0.88f), c, w);
        dl->AddLine(p(0.14f, 0.7f), p(0.5f, 0.88f), c, w);
        dl->AddLine(p(0.86f, 0.7f), p(0.5f, 0.88f), c, w);
    } else if (!std::strcmp(name, "bell")) {
        dl->PathArcTo(p(0.5f, 0.46f), s * 0.26f, PI, 2 * PI, 16);
        dl->PathLineTo(p(0.76f, 0.7f));
        dl->PathLineTo(p(0.24f, 0.7f));
        dl->PathFillConvex(c);
        dl->AddLine(p(0.16f, 0.7f), p(0.84f, 0.7f), c, w);
        dl->AddCircleFilled(p(0.5f, 0.82f), s * 0.08f, c);
    } else if (!std::strcmp(name, "close")) {
        dl->AddLine(p(0.24f, 0.24f), p(0.76f, 0.76f), c, w * 1.2f);
        dl->AddLine(p(0.76f, 0.24f), p(0.24f, 0.76f), c, w * 1.2f);
    }
}

void draw_button(ImDrawList* dl, const Button& b, ImVec2 d, ImVec2 mouse, float u) {
    ImFont* font = ImGui::GetFont();
    ImVec2 a = corner(b, d), z = far(b, d);
    bool hover = inside(b, mouse, d);
    float r = (z.y - a.y) / 2;
    ImU32 fg = TEXT;
    if (b.kind == PRIMARY) {
        // the launcher's Play: filled with the accent, lifted on hover
        if (hover)
            a.y -= 2 * u, z.y -= 2 * u;
        for (int i = 3; i > 0; i--)
            dl->AddRectFilled(ImVec2(a.x - i * 3 * u, a.y + i * 2 * u), ImVec2(z.x + i * 3 * u, z.y + i * 5 * u), IM_COL32(79, 216, 196, 12), r + i * 3 * u);
        dl->AddRectFilled(a, z, ACCENT, r);
        fg = DEEP;
    } else {
        dl->AddRectFilled(a, z, hover ? IM_COL32(22, 32, 40, 235) : PANEL, r);
        dl->AddRect(a, z, hover ? ACCENT : LINE, r, 0, std::max(1.f, u));
        if (b.kind == ICON && !std::strcmp(b.icon, "close") && hover)
            fg = IM_COL32(232, 120, 120, 255);
    }
    float h = z.y - a.y, size = std::round(h * (b.kind == PRIMARY ? 0.36f : 0.34f)), is = std::round(h * 0.38f);
    if (!b.label) {
        icon(dl, b.icon, ImVec2(std::round((a.x + z.x - is) / 2), std::round((a.y + z.y - is) / 2)), is, fg);
        return;
    }
    ImVec2 ts = font->CalcTextSizeA(size, FLT_MAX, 0, b.label);
    float gap = h * 0.22f, x = std::round(a.x + h * 0.42f), mid = (a.y + z.y) / 2;
    icon(dl, b.icon, ImVec2(x, std::round(mid - is / 2)), is, fg);
    dl->AddText(font, size, ImVec2(std::round(x + is + gap), std::round(mid - ts.y / 2)), fg, b.label);
}

// The launcher's sea along the bottom: three layers drifting at their own speeds
void sea(ImDrawList* dl, ImVec2 d, float time) {
    const float heights[] = {0.17f, 0.13f, 0.095f}, speeds[] = {0.05f, -0.09f, 0.14f};
    const int alphas[] = {18, 26, 41};
    for (int l = 0; l < 3; l++) {
        float top = d.y * (1 - heights[l]), amp = d.y * 0.018f, phase = time * speeds[l] * 2 * PI;
        dl->PathLineTo(ImVec2(0, d.y));
        for (int i = 0; i <= 48; i++) {
            float x = d.x * i / 48;
            dl->PathLineTo(ImVec2(x, top + amp * std::sin(x / d.x * 4 * PI + phase)));
        }
        dl->PathLineTo(ImVec2(d.x, d.y));
        dl->PathFillConcave(IM_COL32(79, 216, 196, alphas[l]));
    }
}

}  // namespace

bool showing() {
    GLint vp[4] = {};
    glGetIntegerv(GL_VIEWPORT, vp);
    unsigned char px[4] = {};
    // the mark is the pack's 2px square in the top-left corner: 1, 254, 3 on Talaz's
    // screen, 3, 1, 254 on the game's own
    glReadPixels(vp[0] + 1, vp[1] + vp[3] - 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
    int away = int(std::ceil(14.f * vp[3] / 720)) + 2;
    glReadPixels(vp[0] + away, vp[1] + vp[3] - 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, beside);
    vanilla = px[0] < 12 && px[1] < 12 && px[2] > 240;
    if (vanilla)
        return true;
    if (!(px[0] < 12 && px[1] > 240 && px[2] < 14))
        return false;
    // the second mark sits mid-screen, where a dialog like the quit prompt covers it
    int mx = vp[0] + vp[2] / 2 + 1, my = vp[1] + int(vp[3] * 0.44f) - 2;
    glReadPixels(mx, my, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
    glReadPixels(mx + away, my, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, beside_middle);
    return px[0] < 12 && px[1] > 240 && px[2] < 14;
}

bool owns(ImVec2 point, ImVec2 display) {
    if (vanilla)
        return false;
    return std::any_of(std::begin(buttons), std::end(buttons), [&](const Button& b) { return b.mods && inside(b, point, display); });
}

bool draw(ImDrawList* dl, ImVec2 d, ImVec2 mouse, bool clicked, float time) {
    float u = d.y / 720;
    // the marks grow with the game's interface scale, so the paint over them grows with the screen
    float cover = std::ceil(14 * u);
    dl->AddRectFilled(ImVec2(0, 0), ImVec2(cover, cover), IM_COL32(beside[0], beside[1], beside[2], 255));
    if (vanilla) {
        // the way back to Talaz, at the right end of the row
        static const Button back{nullptr, "waves", 0.945f, 0.03f, 0.035f, 0.06f, ICON};
        draw_button(dl, back, d, mouse, u);
        return false;
    }
    float mx = std::round(d.x / 2), my = std::round(d.y * 0.56f);
    dl->AddRectFilled(ImVec2(mx - cover / 2, my - cover / 2), ImVec2(mx + cover, my + cover), IM_COL32(beside_middle[0], beside_middle[1], beside_middle[2], 255));

    // the panorama stays, darkened on the left where the words are
    dl->AddRectFilledMultiColor(ImVec2(0, 0), ImVec2(d.x * 0.55f, d.y), IM_COL32(12, 18, 23, 225), IM_COL32(12, 18, 23, 0),
                                IM_COL32(12, 18, 23, 0), IM_COL32(12, 18, 23, 225));
    sea(dl, d, time);

    // the launcher's hero: the eyebrow, the wordmark, the waves
    ImFont* font = ImGui::GetFont();
    float left = std::round(d.x * 0.07f);
    dl->AddText(font, std::round(13 * u), ImVec2(left, std::round(d.y * 0.25f)), ACCENT, "B E D R O C K   E D I T I O N");
    float big = std::round(72 * u);
    icon(dl, "waves", ImVec2(left, std::round(d.y * 0.3f + big * 0.12f)), big * 0.8f, ACCENT);
    ImFontAtlas* atlas = ImGui::GetIO().Fonts;
    ImFont* large = atlas->Fonts.Size > 1 ? atlas->Fonts[1] : font;
    dl->AddText(large, big, ImVec2(std::round(left + big * 0.95f), std::round(d.y * 0.3f)), TEXT, "talaz");

    bool open_menu = false;
    for (const Button& b : buttons) {
        draw_button(dl, b, d, mouse, u);
        if (clicked && b.mods && inside(b, mouse, d))
            open_menu = true;
    }
    return open_menu;
}

}  // namespace title
