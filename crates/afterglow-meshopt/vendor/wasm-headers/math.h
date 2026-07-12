#pragma once

// Freestanding math.h using clang builtins.
// These inline to native WASM math instructions.

static __inline__ float fabsf(float x) { return __builtin_fabsf(x); }
static __inline__ double fabs(double x) { return __builtin_fabs(x); }
static __inline__ float sqrtf(float x) { return __builtin_sqrtf(x); }
static __inline__ double sqrt(double x) { return __builtin_sqrt(x); }
static __inline__ float floorf(float x) { return __builtin_floorf(x); }
static __inline__ float ceilf(float x) { return __builtin_ceilf(x); }
static __inline__ float frexpf(float x, int *exp) { return __builtin_frexpf(x, exp); }
static __inline__ float ldexpf(float x, int exp) { return __builtin_ldexpf(x, exp); }
static __inline__ float log2f(float x) { return __builtin_log2f(x); }
static __inline__ double sin(double x) { return __builtin_sin(x); }
static __inline__ double cos(double x) { return __builtin_cos(x); }
