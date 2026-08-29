/* C-engine implementation of the neutral brush API (thin aliases). */
#include "brush_engine.h"
#include "mypaint-brush.h"
#include "mypaint-brush-cooperative.h"

void *be_brush_new(void) { return (void *)mypaint_brush_new(); }

void be_brush_free(void *brush) {
    mypaint_brush_unref((MyPaintBrush *)brush);
}

void be_brush_from_defaults(void *brush) {
    mypaint_brush_from_defaults((MyPaintBrush *)brush);
}

int be_brush_from_string(void *brush, const char *json) {
    return mypaint_brush_from_string((MyPaintBrush *)brush, json) ? 1 : 0;
}

void be_brush_new_stroke(void *brush) {
    mypaint_brush_new_stroke((MyPaintBrush *)brush);
}

void be_brush_reset(void *brush) {
    mypaint_brush_reset((MyPaintBrush *)brush);
}

int be_brush_setting_from_cname(const char *cname) {
    return (int)mypaint_brush_setting_from_cname(cname);
}

int be_brush_input_from_cname(const char *cname) {
    return (int)mypaint_brush_input_from_cname(cname);
}

void be_brush_set_base_value(void *brush, int setting, float value) {
    mypaint_brush_set_base_value((MyPaintBrush *)brush,
                                 (MyPaintBrushSetting)setting, value);
}

float be_brush_get_base_value(void *brush, int setting) {
    return mypaint_brush_get_base_value((MyPaintBrush *)brush,
                                        (MyPaintBrushSetting)setting);
}

void be_brush_set_mapping_n(void *brush, int setting, int input, int n) {
    mypaint_brush_set_mapping_n((MyPaintBrush *)brush,
                                (MyPaintBrushSetting)setting,
                                (MyPaintBrushInput)input, n);
}

void be_brush_set_mapping_point(void *brush, int setting, int input,
                                int index, float x, float y) {
    mypaint_brush_set_mapping_point((MyPaintBrush *)brush,
                                    (MyPaintBrushSetting)setting,
                                    (MyPaintBrushInput)input, index, x, y);
}

int be_stroke_start(void *brush, MyPaintSurface *surface, float x, float y,
                    float pressure, float xtilt, float ytilt, double dtime,
                    float viewzoom, float viewrotation, float barrel_rotation,
                    int linear, int dab_budget) {
    return afterglow_brush_stroke_start((MyPaintBrush *)brush, surface, x, y,
                                        pressure, xtilt, ytilt, dtime,
                                        viewzoom, viewrotation,
                                        barrel_rotation, linear, dab_budget);
}

int be_stroke_continue(int dab_budget) {
    return afterglow_brush_stroke_continue(dab_budget);
}

int be_stroke_pending(void) {
    return afterglow_brush_stroke_pending();
}

void be_stroke_cancel(void) {
    afterglow_brush_stroke_cancel();
}

int be_brush_stroke_to(void *brush, MyPaintSurface *surface, float x, float y,
                       float pressure, float xtilt, float ytilt, double dtime,
                       float viewzoom, float viewrotation,
                       float barrel_rotation, int linear) {
    return mypaint_brush_stroke_to((MyPaintBrush *)brush, surface, x, y,
                                   pressure, xtilt, ytilt, dtime, viewzoom,
                                   viewrotation, barrel_rotation, linear);
}
