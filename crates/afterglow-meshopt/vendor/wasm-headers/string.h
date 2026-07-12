#pragma once

// Freestanding implementations using clang builtins.
// These inline to native WASM instructions — no external library needed.

static __inline__ void *memset(void *dst, int c, unsigned long n) {
    return __builtin_memset(dst, c, n);
}
static __inline__ void *memcpy(void *dst, const void *src, unsigned long n) {
    return __builtin_memcpy(dst, src, n);
}
static __inline__ void *memmove(void *dst, const void *src, unsigned long n) {
    return __builtin_memmove(dst, src, n);
}
static __inline__ int memcmp(const void *a, const void *b, unsigned long n) {
    return __builtin_memcmp(a, b, n);
}
