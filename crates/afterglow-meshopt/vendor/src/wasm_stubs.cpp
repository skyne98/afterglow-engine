// Freestanding WASM stubs — provides missing C/C++ runtime functions.
//
// meshoptimizer uses memset, memcpy, sqrtf, fabsf (C library), and
// operator new/delete (C++ runtime). In freestanding WASM, these aren't
// available. We implement them using clang builtins and a simple bump allocator.

#include <stddef.h>
#include <stdint.h>

// --- C library functions (using clang builtins) ---

extern "C" void *memset(void *dst, int c, size_t n) {
    return __builtin_memset(dst, c, n);
}

extern "C" void *memcpy(void *dst, const void *src, size_t n) {
    return __builtin_memcpy(dst, src, n);
}

extern "C" void *memmove(void *dst, const void *src, size_t n) {
    return __builtin_memmove(dst, src, n);
}

extern "C" int memcmp(const void *a, const void *b, size_t n) {
    return __builtin_memcmp(a, b, n);
}

extern "C" float sqrtf(float x) { return __builtin_sqrtf(x); }
extern "C" double sqrt(double x) { return __builtin_sqrt(x); }
extern "C" float fabsf(float x) { return __builtin_fabsf(x); }
extern "C" double fabs(double x) { return __builtin_fabs(x); }
extern "C" float floorf(float x) { return __builtin_floorf(x); }
extern "C" float ceilf(float x) { return __builtin_ceilf(x); }
extern "C" float frexpf(float x, int *exp) { return __builtin_frexpf(x, exp); }
extern "C" float ldexpf(float x, int exp) { return __builtin_ldexpf(x, exp); }
extern "C" float log2f(float x) { return __builtin_log2f(x); }
extern "C" double sin(double x) { return __builtin_sin(x); }
extern "C" double cos(double x) { return __builtin_cos(x); }

// --- C++ new/delete (meshopt uses new/delete internally) ---
// We redirect to malloc/free which the WASM runtime provides via the
// Rust allocator (the wasm module exports memory managed by Rust).

extern "C" void *malloc(size_t size);
extern "C" void free(void *ptr);

void *operator new(size_t size) { return malloc(size); }
void *operator new[](size_t size) { return malloc(size); }
void operator delete(void *ptr) noexcept { free(ptr); }
void operator delete[](void *ptr) noexcept { free(ptr); }
void operator delete(void *ptr, size_t) noexcept { free(ptr); }
void operator delete[](void *ptr, size_t) noexcept { free(ptr); }
