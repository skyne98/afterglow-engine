#pragma once
#include <stddef.h>
static __inline__ void *memset(void *d, int c, size_t n) { return __builtin_memset(d,c,n); }
static __inline__ void *memcpy(void *d, const void *s, size_t n) { return __builtin_memcpy(d,s,n); }
static __inline__ void *memmove(void *d, const void *s, size_t n) { return __builtin_memmove(d,s,n); }
static __inline__ int memcmp(const void *a, const void *b, size_t n) { return __builtin_memcmp(a,b,n); }
