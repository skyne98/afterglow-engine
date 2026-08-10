#include <stdio.h>
#include <string.h>
#include <emscripten.h>
#include "mypaint-config.h"
#include "mypaint.h"
#include "mypaint-brush.h"
#include "mypaint-brush-settings.h"
#include "mypaint-mapping.h"
#include "mypaint-surface.h"
#include "proxy-surface.h"

static ProxySurface *surface;
static MyPaintBrush *brush;
static int init_done = 0;

/* Public primitive API, mirroring eliot-akira's brushlib-wasm wrapper.
 * These are the leaf operations the JS side calls; forward-declared to keep
 * them exported with the exact names emscripten emits.
 */

void new_brush(void)
{
    if (!init_done) {
        mypaint_init();
        init_done = 1;
    }
    if (brush != NULL)
        mypaint_brush_unref(brush);
    brush = mypaint_brush_new();
    mypaint_brush_from_defaults(brush);
    mypaint_brush_new_stroke(brush);
}

int load_brush(const char *brush_json)
{
    new_brush();
    const int loaded = mypaint_brush_from_string(brush, brush_json) ? 1 : 0;
    if (loaded)
        mypaint_brush_new_stroke(brush);
    return loaded;
}

void begin_stroke(float x, float y, float xtilt, float ytilt,
                  float viewzoom, float viewrotation, float barrel_rotation)
{
    if (brush != NULL) {
        mypaint_brush_reset(brush);
        mypaint_brush_new_stroke(brush);
        mypaint_brush_stroke_to(brush, (MyPaintSurface *)surface, x, y, 0.0f,
                                xtilt, ytilt, 10.0, viewzoom, viewrotation,
                                barrel_rotation, 0);
    }
}

void set_brush_base_value(const char *setting_name, double base_value)
{
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    mypaint_brush_set_base_value(brush, setting_id, (float)base_value);
}

void set_brush_mapping_n(const char *setting_name, const char *input_name, int number_of_mapping_points)
{
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    MyPaintBrushInput input_id = mypaint_brush_input_from_cname(input_name);
    mypaint_brush_set_mapping_n(brush, setting_id, input_id, number_of_mapping_points);
}

void set_brush_mapping_point(const char *setting_name, const char *input_name, int index, float x, float y)
{
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    MyPaintBrushInput input_id = mypaint_brush_input_from_cname(input_name);
    mypaint_brush_set_mapping_point(brush, setting_id, input_id, index, x, y);
}

void reset_brush(void)
{
    mypaint_brush_reset(brush);
}

void stroke_to(float x, float y, float pressure, float xtilt, float ytilt, double dtime,
               float viewzoom, float viewrotation, float barrel_rotation)
{
    mypaint_brush_stroke_to(brush, (MyPaintSurface *)surface, x, y, pressure,
                            xtilt, ytilt, dtime, viewzoom, viewrotation,
                            barrel_rotation, 0); /* linear = 0 */
}

void init(DrawDabFunctionCallback draw_dab_cb, GetColorFunctionCallback get_color_cb)
{
    if (!init_done) {
        mypaint_init();
        init_done = 1;
    }
    surface = proxy_surface_new(draw_dab_cb, get_color_cb);
    brush = mypaint_brush_new();
}
