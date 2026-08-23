#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mypaint.h"
#include "mypaint-brush.h"
#include "mypaint-brush-cooperative.h"
#include "mypaint-brush-settings.h"
#include "mypaint-surface.h"
#include "web-surface.h"

static void set_value(MyPaintBrush *brush, const char *name, float value)
{
    mypaint_brush_set_base_value(
        brush, mypaint_brush_setting_from_cname(name), value);
}

static void configure(MyPaintBrush *brush)
{
    mypaint_brush_from_defaults(brush);
    set_value(brush, "radius_logarithmic", logf(8.0f));
    set_value(brush, "dabs_per_actual_radius", 20.0f);
    set_value(brush, "dabs_per_basic_radius", 2.0f);
    set_value(brush, "opaque", 0.8f);
    set_value(brush, "opaque_multiply", 1.0f);
    set_value(brush, "hardness", 0.7f);
    set_value(brush, "color_h", 0.7f);
    set_value(brush, "color_s", 0.6f);
    set_value(brush, "color_v", 0.5f);
}

static void start_brush(MyPaintBrush *brush, WebPaintSurface *surface)
{
    mypaint_brush_reset(brush);
    mypaint_brush_new_stroke(brush);
    mypaint_surface_begin_atomic(web_surface_interface(surface));
    mypaint_brush_stroke_to(
        brush, web_surface_interface(surface), 100.0f, 300.0f, 0.0f,
        0.0f, 0.0f, 10.0, 1.0f, 0.0f, 0.0f, 0);
    mypaint_surface_end_atomic(web_surface_interface(surface), NULL);
}

static int paint_standard(MyPaintBrush *brush, WebPaintSurface *surface)
{
    mypaint_surface_begin_atomic(web_surface_interface(surface));
    for (int i = 1; i <= 4; i++) {
        mypaint_brush_stroke_to(
            brush, web_surface_interface(surface), 100.0f + i * 180.0f,
            300.0f + i * 20.0f, 0.65f, 0.1f, -0.15f, 0.016,
            1.0f, 0.0f, 0.25f, 0);
    }
    mypaint_surface_end_atomic(web_surface_interface(surface), NULL);
    return 1;
}

static int paint_cooperative(MyPaintBrush *brush, WebPaintSurface *surface,
                             int *continuations)
{
    mypaint_surface_begin_atomic(web_surface_interface(surface));
    for (int i = 1; i <= 4; i++) {
        int result = afterglow_brush_stroke_start(
            brush, web_surface_interface(surface), 100.0f + i * 180.0f,
            300.0f + i * 20.0f, 0.65f, 0.1f, -0.15f, 0.016,
            1.0f, 0.0f, 0.25f, 0, 3);
        while (result == 0) {
            (*continuations)++;
            result = afterglow_brush_stroke_continue(3);
        }
        if (result < 0) return 0;
    }
    mypaint_surface_end_atomic(web_surface_interface(surface), NULL);
    return 1;
}

static int compare_surfaces(WebPaintSurface *a, WebPaintSurface *b)
{
    if (web_surface_get_used_tile_count(a) !=
        web_surface_get_used_tile_count(b)) return 0;
    const size_t bytes = 64u * 64u * 4u * sizeof(uint16_t);
    for (int i = 0; i < web_surface_get_used_tile_count(a); i++) {
        int tx = 0;
        int ty = 0;
        if (!web_surface_get_used_tile_info(a, i, &tx, &ty)) return 0;
        uint16_t *left = web_surface_get_used_tile(a, i);
        uint16_t *right = web_surface_get_tile(b, tx, ty);
        if (!left || !right || memcmp(left, right, bytes) != 0) return 0;
    }
    return 1;
}

int main(void)
{
    mypaint_init();
    MyPaintBrush *standard = mypaint_brush_new();
    MyPaintBrush *cooperative = mypaint_brush_new();
    WebPaintSurface *standard_surface = web_surface_new(1024, 1024);
    WebPaintSurface *cooperative_surface = web_surface_new(1024, 1024);
    if (!standard || !cooperative || !standard_surface ||
        !cooperative_surface) return 1;

    configure(standard);
    configure(cooperative);
    start_brush(standard, standard_surface);
    start_brush(cooperative, cooperative_surface);
    int continuations = 0;
    const int painted = paint_standard(standard, standard_surface) &&
        paint_cooperative(cooperative, cooperative_surface, &continuations);
    const int surfaces_equal = compare_surfaces(
        standard_surface, cooperative_surface);
    int states_equal = 1;
    for (int i = 0; i < MYPAINT_BRUSH_STATES_COUNT; i++) {
        const float left = mypaint_brush_get_state(
            standard, (MyPaintBrushState)i);
        const float right = mypaint_brush_get_state(
            cooperative, (MyPaintBrushState)i);
        if (left != right) {
            fprintf(stderr, "state %d differs: %a != %a\n",
                    i, (double)left, (double)right);
            states_equal = 0;
        }
    }

    mypaint_brush_unref(standard);
    mypaint_brush_unref(cooperative);
    web_surface_destroy(standard_surface);
    web_surface_destroy(cooperative_surface);
    if (!painted || continuations < 1 || !surfaces_equal || !states_equal) {
        fprintf(stderr,
                "cooperative stroke failed: painted=%d continuations=%d surface=%d state=%d\n",
                painted, continuations, surfaces_equal, states_equal);
        return 1;
    }
    printf("cooperative stroke: exact (%d continuations)\n", continuations);
    return 0;
}
