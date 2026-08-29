/* maipointo (Rust) engine implementation of the neutral brush API.
 * The maipointo staticlib provides the maipo_* C ABI; the cooperative
 * stroke driver lives inside the Rust engine and drives the C surface
 * through libmypaint's public draw_dab/get_color entry points. */
#include "brush_engine.h"

extern void *maipo_brush_new(int num_smudge_buckets);
extern void maipo_brush_free(void *brush);
extern void maipo_brush_from_defaults(void *brush);
extern int maipo_brush_from_string(void *brush, const char *json);
extern void maipo_brush_new_stroke(void *brush);
extern void maipo_brush_reset(void *brush);
extern int maipo_brush_setting_from_cname(const char *cname);
extern int maipo_brush_input_from_cname(const char *cname);
extern void maipo_brush_set_base_value(void *brush, int setting, float value);
extern float maipo_brush_get_base_value(void *brush, int setting);
extern void maipo_brush_set_mapping_n(void *brush, int setting, int input,
                                      int n);
extern void maipo_brush_set_mapping_point(void *brush, int setting,
                                          int input, int index, float x,
                                          float y);
extern int maipo_brush_stroke_start(void *brush, void *surface, float x,
                                    float y, float pressure, float xtilt,
                                    float ytilt, double dtime, float viewzoom,
                                    float viewrotation, float barrel_rotation,
                                    int linear, int dab_budget);
extern int maipo_brush_stroke_continue(int dab_budget);
extern int maipo_brush_stroke_pending(void);
extern void maipo_brush_stroke_cancel(void);

void *be_brush_new(void) { return maipo_brush_new(0); }

void be_brush_free(void *brush) { maipo_brush_free(brush); }

void be_brush_from_defaults(void *brush) { maipo_brush_from_defaults(brush); }

int be_brush_from_string(void *brush, const char *json) {
    return maipo_brush_from_string(brush, json);
}

void be_brush_new_stroke(void *brush) { maipo_brush_new_stroke(brush); }

void be_brush_reset(void *brush) { maipo_brush_reset(brush); }

int be_brush_setting_from_cname(const char *cname) {
    return maipo_brush_setting_from_cname(cname);
}

int be_brush_input_from_cname(const char *cname) {
    return maipo_brush_input_from_cname(cname);
}

void be_brush_set_base_value(void *brush, int setting, float value) {
    maipo_brush_set_base_value(brush, setting, value);
}

float be_brush_get_base_value(void *brush, int setting) {
    return maipo_brush_get_base_value(brush, setting);
}

void be_brush_set_mapping_n(void *brush, int setting, int input, int n) {
    maipo_brush_set_mapping_n(brush, setting, input, n);
}

void be_brush_set_mapping_point(void *brush, int setting, int input,
                                int index, float x, float y) {
    maipo_brush_set_mapping_point(brush, setting, input, index, x, y);
}

int be_stroke_start(void *brush, MyPaintSurface *surface, float x, float y,
                    float pressure, float xtilt, float ytilt, double dtime,
                    float viewzoom, float viewrotation, float barrel_rotation,
                    int linear, int dab_budget) {
    return maipo_brush_stroke_start(brush, (void *)surface, x, y, pressure,
                                    xtilt, ytilt, dtime, viewzoom,
                                    viewrotation, barrel_rotation, linear,
                                    dab_budget);
}

/* The Rust engine keeps its own continuation state, but the C budget loop
 * needs the surface pointer; the Rust side stores it, so pass NULL here and
 * let the maipo module reuse the stored surface. */
int be_stroke_continue(int dab_budget) {
    return maipo_brush_stroke_continue(dab_budget);
}

int be_stroke_pending(void) { return maipo_brush_stroke_pending(); }

void be_stroke_cancel(void) { maipo_brush_stroke_cancel(); }

extern int maipo_brush_stroke_to(void *brush, void *surface, float x, float y,
                                 float pressure, float xtilt, float ytilt,
                                 double dtime, float viewzoom,
                                 float viewrotation, float barrel_rotation,
                                 int linear);

int be_brush_stroke_to(void *brush, MyPaintSurface *surface, float x, float y,
                       float pressure, float xtilt, float ytilt, double dtime,
                       float viewzoom, float viewrotation,
                       float barrel_rotation, int linear) {
    return maipo_brush_stroke_to(brush, (void *)surface, x, y, pressure,
                                 xtilt, ytilt, dtime, viewzoom, viewrotation,
                                 barrel_rotation, linear);
}
