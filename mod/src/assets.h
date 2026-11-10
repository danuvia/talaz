#pragma once

#include <cstddef>

// Fonts are baked into the library so the mod stays one file. FONT_DIR
// comes from CMake.
#define EMBED(name, path)                                                                                    \
    __asm__(".section .rodata\n.balign 16\n" #name ":\n.incbin \"" path "\"\n" #name "_end:\n.byte 0\n.previous\n"); \
    extern "C" const unsigned char name[], name##_end[];

struct Asset {
    const unsigned char* data;
    size_t size;
};

#define ASSET(name) (Asset{name, size_t(name##_end - name)})
