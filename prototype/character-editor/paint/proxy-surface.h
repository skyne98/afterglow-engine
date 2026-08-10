#ifndef PROXY_SURFACE_H
#define PROXY_SURFACE_H

#include "mypaint-surface.h"

struct _ProxySurface;
typedef struct _ProxySurface ProxySurface;

typedef void (*GetColorFunctionCallback)(
    float x, float y, float radius,
    float *color_r, float *color_g, float *color_b, float *color_a);

typedef int (*DrawDabFunctionCallback)(
    float x, float y, float radius,
    float color_r, float color_g, float color_b,
    float opaque, float hardness, float softness,
    float alpha_eraser,
    float aspect_ratio, float angle,
    float lock_alpha,
    float colorize,
    float posterize, float posterize_num,
    float paint);

ProxySurface *proxy_surface_new(
    DrawDabFunctionCallback draw_dab_cb,
    GetColorFunctionCallback get_color_cb);

struct _ProxySurface {
    MyPaintSurface parent;
    GetColorFunctionCallback get_color_cb;
    DrawDabFunctionCallback draw_dab_cb;

};

#endif // PROXY_SURFACE_H
