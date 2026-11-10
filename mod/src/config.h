#pragma once

#include <string>
#include <vector>

// Every setting the mod has, as named numbers kept in files next to the mod
// library. Mod settings belong to the active profile, the rest (menu
// preferences, theme, keybinds) are shared by all profiles.
namespace config {

// Declares a setting and its default. Every key is declared at startup, before
// input starts arriving, so lookups never insert while another thread reads.
void declare(const std::string& key, float value);

float get(const std::string& key);
void set(const std::string& key, float value);
inline bool on(const std::string& key) { return get(key) != 0; }

void load();
void save();

// Profiles: named sets of mod settings
std::vector<std::string> profiles();
const std::string& profile();
void switch_profile(const std::string& name);
// Copies the current settings into a new profile and switches to it
void create_profile(const std::string& name);
void rename_profile(const std::string& from, const std::string& to);
void delete_profile(const std::string& name);
// A value from another profile's file, without switching to it
float profile_value(const std::string& name, const std::string& key, float fallback);
void set_profile_value(const std::string& name, const std::string& key, float value);

}  // namespace config
