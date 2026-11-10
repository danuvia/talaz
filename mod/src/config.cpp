#include "config.h"

#include <dirent.h>
#include <dlfcn.h>

#include <algorithm>
#include <cstdio>
#include <map>

namespace config {
namespace {

constexpr const char* DEFAULT_PROFILE = "Default Profile";

std::map<std::string, float> values, defaults;
std::string active = DEFAULT_PROFILE;

bool global(const std::string& key) {
    return key.rfind("pref.", 0) == 0 || key.rfind("theme.", 0) == 0 || key.rfind("key.", 0) == 0;
}

// <dir>/libtalaz_bedrock.so keeps its files beside it:
// libtalaz_bedrock.ini and libtalaz_bedrock.profile.<name>.ini
std::string base() {
    Dl_info info{};
    if (!dladdr(reinterpret_cast<void*>(&base), &info) || !info.dli_fname)
        return {};
    std::string path = info.dli_fname;
    size_t dot = path.rfind(".so");
    return dot == std::string::npos ? path : path.substr(0, dot);
}

std::string profile_path(const std::string& name) { return base() + ".profile." + name + ".ini"; }

void read(const std::string& path, bool want_global) {
    FILE* f = std::fopen(path.c_str(), "r");
    if (!f)
        return;
    char line[512];
    while (std::fgets(line, sizeof line, f)) {
        std::string s = line;
        size_t eq = s.find('=');
        if (eq == std::string::npos)
            continue;
        std::string key = s.substr(0, eq), value = s.substr(eq + 1);
        while (!value.empty() && (value.back() == '\n' || value.back() == '\r'))
            value.pop_back();
        if (key == "profile" && want_global) {
            active = value;
            continue;
        }
        if (global(key) != want_global || !values.count(key))
            continue;
        values[key] = std::strtof(value.c_str(), nullptr);
    }
    std::fclose(f);
}

void write(const std::string& path, bool want_global) {
    FILE* f = std::fopen(path.c_str(), "w");
    if (!f)
        return;
    if (want_global)
        std::fprintf(f, "profile=%s\n", active.c_str());
    for (auto& [key, value] : values)
        if (global(key) == want_global)
            std::fprintf(f, "%s=%.9g\n", key.c_str(), value);
    std::fclose(f);
}

void reset_profile_values() {
    for (auto& [key, value] : values)
        if (!global(key))
            value = defaults[key];
}

}  // namespace

void declare(const std::string& key, float value) {
    values.try_emplace(key, value);
    defaults[key] = value;
}

float get(const std::string& key) {
    auto it = values.find(key);
    return it == values.end() ? 0 : it->second;
}

void set(const std::string& key, float value) {
    auto it = values.find(key);
    if (it == values.end() || it->second == value)
        return;
    it->second = value;
    save();
}

void load() {
    if (base().empty())
        return;
    read(base() + ".ini", true);
    read(profile_path(active), false);
}

void save() {
    if (base().empty())
        return;
    write(base() + ".ini", true);
    write(profile_path(active), false);
}

std::vector<std::string> profiles() {
    std::string b = base();
    std::vector<std::string> out;
    size_t slash = b.rfind('/');
    std::string dir = slash == std::string::npos ? "." : b.substr(0, slash);
    std::string prefix = b.substr(slash + 1) + ".profile.";
    if (DIR* d = opendir(dir.c_str())) {
        while (dirent* e = readdir(d)) {
            std::string name = e->d_name;
            if (name.rfind(prefix, 0) == 0 && name.size() > prefix.size() + 4 && name.substr(name.size() - 4) == ".ini")
                out.push_back(name.substr(prefix.size(), name.size() - prefix.size() - 4));
        }
        closedir(d);
    }
    if (std::find(out.begin(), out.end(), active) == out.end())
        out.push_back(active);
    std::sort(out.begin(), out.end());
    return out;
}

const std::string& profile() { return active; }

void switch_profile(const std::string& name) {
    if (name == active)
        return;
    save();
    active = name;
    reset_profile_values();
    read(profile_path(active), false);
    save();
}

void create_profile(const std::string& name) {
    save();
    active = name;
    save();
}

void rename_profile(const std::string& from, const std::string& to) {
    if (from == to || to.empty())
        return;
    std::rename(profile_path(from).c_str(), profile_path(to).c_str());
    if (active == from) {
        active = to;
        save();
    }
}

void delete_profile(const std::string& name) {
    std::remove(profile_path(name).c_str());
    if (active == name) {
        std::vector<std::string> left = profiles();
        left.erase(std::remove(left.begin(), left.end(), name), left.end());
        // the deleted profile's values are dropped, not saved into the next one
        active = left.empty() ? DEFAULT_PROFILE : left.front();
        reset_profile_values();
        read(profile_path(active), false);
        save();
    }
}

float profile_value(const std::string& name, const std::string& key, float fallback) {
    if (name == active)
        return values.count(key) ? values[key] : fallback;
    FILE* f = std::fopen(profile_path(name).c_str(), "r");
    if (!f)
        return fallback;
    float out = fallback;
    char line[512];
    std::string prefix = key + "=";
    while (std::fgets(line, sizeof line, f))
        if (std::string(line).rfind(prefix, 0) == 0)
            out = std::strtof(line + prefix.size(), nullptr);
    std::fclose(f);
    return out;
}

void set_profile_value(const std::string& name, const std::string& key, float value) {
    if (name == active)
        return set(key, value);
    std::string path = profile_path(name), text, line_text;
    if (FILE* f = std::fopen(path.c_str(), "r")) {
        char line[512];
        while (std::fgets(line, sizeof line, f))
            if (std::string(line).rfind(key + "=", 0) != 0)
                text += line;
        std::fclose(f);
    }
    char buf[64];
    std::snprintf(buf, sizeof buf, "%.9g", value);
    text += key + "=" + buf + "\n";
    if (FILE* f = std::fopen(path.c_str(), "w")) {
        std::fputs(text.c_str(), f);
        std::fclose(f);
    }
}

}  // namespace config
