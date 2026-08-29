#ifndef AFTERGLOW_BRUSH_ENGINE_H
#define AFTERGLOW_BRUSH_ENGINE_H

/* Engine-neutral brush API for the paint demo. Two implementations exist:
 *   brush_engine_c.c    - the upstream libmypaint C engine
 *   brush_engine_rust.c - the maipointo Rust port (maipointo staticlib)
 * main.c talks only to this header; build-wasm.sh picks the engine.
 */

#include "mypaint-surface.h"

void *be_brush_new(void);
void be_brush_free(void *brush);
void be_brush_from_defaults(void *brush);
int be_brush_from_string(void *brush, const char *json);
void be_brush_new_stroke(void *brush);
void be_brush_reset(void *brush);
int be_brush_setting_from_cname(const char *cname);
int be_brush_input_from_cname(const char *cname);
void be_brush_set_base_value(void *brush, int setting, float value);
float be_brush_get_base_value(void *brush, int setting);
void be_brush_set_mapping_n(void *brush, int setting, int input, int n);
void be_brush_set_mapping_point(void *brush, int setting, int input,
                                int index, float x, float y);

/* Unbudgeted direct stroke_to (used for the zero-pressure warm-up). */
int be_brush_stroke_to(void *brush, MyPaintSurface *surface, float x, float y,
                       float pressure, float xtilt, float ytilt, double dtime,
                       float viewzoom, float viewrotation,
                       float barrel_rotation, int linear);

/* Cooperative budgeted stroke (dab_budget semantics identical across
 * engines; returns 2 = caller should split, 1 = queued/finished, 0 =
 * budget exhausted with work pending, negative = error). */
int be_stroke_start(void *brush, MyPaintSurface *surface, float x, float y,
                    float pressure, float xtilt, float ytilt, double dtime,
                    float viewzoom, float viewrotation, float barrel_rotation,
                    int linear, int dab_budget);
int be_stroke_continue(int dab_budget);
int be_stroke_pending(void);
void be_stroke_cancel(void);

#endif
