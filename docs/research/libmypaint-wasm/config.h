/* Minimal generated config.h matching what libmypaint master needs,
 * without autotools. We build the GLib-compat (no real GLib) path.
 */
#ifndef MYPAINT_CONFIG_H
#define MYPAINT_CONFIG_H

/* Build without real GLib: use mypaint-glib-compat shim types. */
#define MYPAINT_CONFIG_USE_GLIB 0

/* Hint to silence some compilers. */
#if defined(_MSC_VER) || defined(WIN32)
#define MYPAINT_OS_WIN32 1
#else
#define MYPAINT_OS_WIN32 0
#endif

#endif /* MYPAINT_CONFIG_H */
