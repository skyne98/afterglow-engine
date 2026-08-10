#include <emscripten.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "mypaint-config.h"
#include "mypaint.h"
#include "mypaint-brush.h"
#include "mypaint-brush-settings.h"
#include "mypaint-mapping.h"
#include "helpers.h"
#include "mypaint-surface.h"
#include "mypaint-symmetry.h"
#include "web-surface.h"
#include "layer-compositor.h"

#define WEB_MAX_LAYERS 8
#define WEB_MAX_GROUPS 4
#define WEB_HISTORY_RECORDS 40
#define WEB_MIP_MAX_SOURCES 16
#define DISPLAY_LUT_VALUES 32769
#define DISPLAY_LUT_NOISE 256

static WebPaintSurface *layers[WEB_MAX_LAYERS];
static WebPaintSurface *background_surface;
static uint16_t background_color[4];
static uint8_t layer_visible[WEB_MAX_LAYERS];
static float layer_opacity[WEB_MAX_LAYERS];
static int layer_mode[WEB_MAX_LAYERS];
static int layer_parent[WEB_MAX_LAYERS];
static int layer_next[WEB_MAX_LAYERS];
static int layer_previous[WEB_MAX_LAYERS];
static int layer_count;
static uint8_t group_alive[WEB_MAX_GROUPS];
static uint8_t group_visible[WEB_MAX_GROUPS];
static uint8_t group_pass_through[WEB_MAX_GROUPS];
static uint8_t group_isolated[WEB_MAX_GROUPS];
static float group_opacity[WEB_MAX_GROUPS];
static int group_mode[WEB_MAX_GROUPS];
static int group_parent[WEB_MAX_GROUPS];
static int group_next[WEB_MAX_GROUPS];
static int group_previous[WEB_MAX_GROUPS];
static int group_first_child[WEB_MAX_GROUPS];
static int group_last_child[WEB_MAX_GROUPS];
static int group_count;
static int root_first_child;
static int root_last_child;
static uint16_t *group_tile[WEB_MAX_GROUPS];
static uint16_t *group_base_tile[WEB_MAX_GROUPS];
static int active_layer;
static WebPaintSurface *surface;
static MyPaintBrush *brush;
static uint16_t *composite_tile;
static uint16_t *mip_composite_tile;
static uint16_t *mip_source_tiles;
static uint16_t *background_tile;
static uint8_t *display_tile;
static uint8_t *display_lut;
static int display_lut_ready;
static size_t tile_bytes;
static float display_eotf = 2.2f;
static uint16_t *history_before;
static uint16_t *history_after;
static int *history_tx;
static int *history_ty;
static int history_capacity;
static int history_record_start[WEB_HISTORY_RECORDS];
static int history_record_count[WEB_HISTORY_RECORDS];
static int history_record_layer[WEB_HISTORY_RECORDS];
static int history_entry_count;
static int history_record_total;
static int history_cursor;
static int history_active;
static int history_active_start;
static int history_active_count;
static int history_active_layer;
static int init_done = 0;
static int atomic_active = 0;
static int suppress_atomic_end = 0;
static int paint_error_code = 0;

static void history_capture_before(WebPaintSurface *owner, int tx, int ty,
                                   uint16_t *tile);
static int history_ensure(int needed);
static void history_free(void);

#define WEB_REF_NONE (-1000000)
#define WEB_REF_GROUP(group_id) (-(group_id) - 1)
#define WEB_REF_IS_GROUP(ref) ((ref) < 0 && (ref) != WEB_REF_NONE)
#define WEB_REF_GROUP_ID(ref) (-(ref) - 1)

static MyPaintRectangle dirty_rects[WEB_SURFACE_MAX_DIRTY_RECTS];
static MyPaintRectangles dirty_roi = {
    WEB_SURFACE_MAX_DIRTY_RECTS,
    dirty_rects,
};
static MyPaintRectangle atomic_dirty_rects[WEB_SURFACE_MAX_DIRTY_RECTS];
static MyPaintRectangles atomic_dirty_roi = {
    WEB_SURFACE_MAX_DIRTY_RECTS,
    atomic_dirty_rects,
};

static float clamp01(float value)
{
    return value < 0.0f ? 0.0f : value > 1.0f ? 1.0f : value;
}

void paint_set_background_color(float r, float g, float b);
static void rebuild_display_lut(void);
void paint_history_begin(void);
void paint_history_commit(void);

static void ensure_init(void)
{
    if (!init_done) {
        mypaint_init();
        init_done = 1;
    }
}

static MyPaintSurface *surface_interface(void)
{
    return surface ? web_surface_interface(surface) : NULL;
}

static void mark_full_dirty(void)
{
    if (!surface) {
        dirty_roi.num_rectangles = 0;
        return;
    }
    dirty_rects[0].x = 0;
    dirty_rects[0].y = 0;
    dirty_rects[0].width = web_surface_get_width(surface);
    dirty_rects[0].height = web_surface_get_height(surface);
    dirty_roi.num_rectangles = 1;
}

static void begin_atomic_internal(void)
{
    if (surface && !atomic_active) {
        mypaint_surface_begin_atomic(surface_interface());
        atomic_active = 1;
    }
}

static void merge_dirty_rectangle(const MyPaintRectangle *source)
{
    if (!source || source->width <= 0 || source->height <= 0) return;
    if (dirty_roi.num_rectangles == 0) {
        dirty_rects[0] = *source;
        dirty_roi.num_rectangles = 1;
        return;
    }
    MyPaintRectangle *target = &dirty_rects[0];
    const int left = target->x < source->x ? target->x : source->x;
    const int top = target->y < source->y ? target->y : source->y;
    const int target_right = target->x + target->width;
    const int source_right = source->x + source->width;
    const int target_bottom = target->y + target->height;
    const int source_bottom = source->y + source->height;
    const int right = target_right > source_right ? target_right : source_right;
    const int bottom = target_bottom > source_bottom ? target_bottom : source_bottom;
    target->x = left;
    target->y = top;
    target->width = right - left;
    target->height = bottom - top;
    dirty_roi.num_rectangles = 1;
}

static int end_atomic_internal(void)
{
    if (!surface || !atomic_active) {
        return dirty_roi.num_rectangles;
    }
    atomic_dirty_roi.num_rectangles = WEB_SURFACE_MAX_DIRTY_RECTS;
    mypaint_surface_end_atomic(surface_interface(), &atomic_dirty_roi);
    for (int i = 0; i < atomic_dirty_roi.num_rectangles; i++) {
        merge_dirty_rectangle(&atomic_dirty_rects[i]);
    }
    if (web_surface_take_capacity_error(surface)) {
        paint_error_code = 1;
    }
    atomic_active = 0;
    return dirty_roi.num_rectangles;
}

static int node_parent(int ref)
{
    if (WEB_REF_IS_GROUP(ref)) return group_parent[WEB_REF_GROUP_ID(ref)];
    if (ref >= 0 && ref < WEB_MAX_LAYERS) return layer_parent[ref];
    return -2;
}

static int node_next(int ref)
{
    if (WEB_REF_IS_GROUP(ref)) return group_next[WEB_REF_GROUP_ID(ref)];
    if (ref >= 0 && ref < WEB_MAX_LAYERS) return layer_next[ref];
    return WEB_REF_NONE;
}

static int node_previous(int ref)
{
    if (WEB_REF_IS_GROUP(ref)) return group_previous[WEB_REF_GROUP_ID(ref)];
    if (ref >= 0 && ref < WEB_MAX_LAYERS) return layer_previous[ref];
    return WEB_REF_NONE;
}

static void node_set_parent(int ref, int parent)
{
    if (WEB_REF_IS_GROUP(ref)) group_parent[WEB_REF_GROUP_ID(ref)] = parent;
    else if (ref >= 0 && ref < WEB_MAX_LAYERS) layer_parent[ref] = parent;
}

static void node_set_next(int ref, int next)
{
    if (WEB_REF_IS_GROUP(ref)) group_next[WEB_REF_GROUP_ID(ref)] = next;
    else if (ref >= 0 && ref < WEB_MAX_LAYERS) layer_next[ref] = next;
}

static void node_set_previous(int ref, int previous)
{
    if (WEB_REF_IS_GROUP(ref)) group_previous[WEB_REF_GROUP_ID(ref)] = previous;
    else if (ref >= 0 && ref < WEB_MAX_LAYERS) layer_previous[ref] = previous;
}

static int node_first(int parent)
{
    return parent < 0 ? root_first_child : group_first_child[parent];
}

static int node_last(int parent)
{
    return parent < 0 ? root_last_child : group_last_child[parent];
}

static void node_set_first(int parent, int ref)
{
    if (parent < 0) root_first_child = ref;
    else group_first_child[parent] = ref;
}

static void node_set_last(int parent, int ref)
{
    if (parent < 0) root_last_child = ref;
    else group_last_child[parent] = ref;
}

static void node_remove(int ref)
{
    const int parent = node_parent(ref);
    if (parent < -1) return;
    const int previous = node_previous(ref);
    const int next = node_next(ref);
    if (previous == WEB_REF_NONE) node_set_first(parent, next);
    else node_set_next(previous, next);
    if (next == WEB_REF_NONE) node_set_last(parent, previous);
    else node_set_previous(next, previous);
    node_set_parent(ref, -2);
    node_set_previous(ref, WEB_REF_NONE);
    node_set_next(ref, WEB_REF_NONE);
}

static void node_append(int ref, int parent)
{
    const int last = node_last(parent);
    node_set_parent(ref, parent);
    node_set_previous(ref, last);
    node_set_next(ref, WEB_REF_NONE);
    if (last == WEB_REF_NONE) node_set_first(parent, ref);
    else node_set_next(last, ref);
    node_set_last(parent, ref);
}

static void node_insert_before(int ref, int before)
{
    const int parent = node_parent(before);
    const int previous = node_previous(before);
    node_set_parent(ref, parent);
    node_set_previous(ref, previous);
    node_set_next(ref, before);
    node_set_previous(before, ref);
    if (previous == WEB_REF_NONE) node_set_first(parent, ref);
    else node_set_next(previous, ref);
}

static int group_contains(int ancestor, int candidate)
{
    int current = candidate;
    while (current >= 0 && current < WEB_MAX_GROUPS) {
        if (current == ancestor) return 1;
        current = group_parent[current];
    }
    return 0;
}

static void reset_tree(void)
{
    root_first_child = WEB_REF_NONE;
    root_last_child = WEB_REF_NONE;
    group_count = 0;
    for (int i = 0; i < WEB_MAX_LAYERS; i++) {
        layer_parent[i] = -2;
        layer_next[i] = WEB_REF_NONE;
        layer_previous[i] = WEB_REF_NONE;
    }
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        group_alive[i] = 0;
        group_visible[i] = 0;
        group_pass_through[i] = 0;
        group_isolated[i] = 1;
        group_opacity[i] = 1.0f;
        group_mode[i] = WEB_MODE_NORMAL;
        group_parent[i] = -2;
        group_next[i] = WEB_REF_NONE;
        group_previous[i] = WEB_REF_NONE;
        group_first_child[i] = WEB_REF_NONE;
        group_last_child[i] = WEB_REF_NONE;
    }
}

static void destroy_layers(void)
{
    if (background_surface) {
        web_surface_destroy(background_surface);
        background_surface = NULL;
    }
    for (int i = 0; i < WEB_MAX_LAYERS; i++) {
        if (layers[i]) {
            web_surface_destroy(layers[i]);
            layers[i] = NULL;
        }
        layer_visible[i] = 0;
        layer_opacity[i] = 0.0f;
        layer_mode[i] = WEB_MODE_PIGMENT;
    }
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        free(group_tile[i]);
        free(group_base_tile[i]);
        group_tile[i] = NULL;
        group_base_tile[i] = NULL;
    }
    reset_tree();
    memset(background_color, 0, sizeof(background_color));
    layer_count = 0;
    active_layer = 0;
    surface = NULL;
}

void new_brush(void)
{
    ensure_init();
    if (brush) {
        mypaint_brush_unref(brush);
    }
    brush = mypaint_brush_new();
    mypaint_brush_from_defaults(brush);
    mypaint_brush_new_stroke(brush);
}

int load_brush(const char *brush_json)
{
    if (!brush_json) {
        return 0;
    }
    new_brush();
    const int loaded = mypaint_brush_from_string(brush, brush_json) ? 1 : 0;
    if (loaded) {
        mypaint_brush_new_stroke(brush);
    }
    return loaded;
}

int init(int width, int height)
{
    ensure_init();
    if (atomic_active) {
        end_atomic_internal();
    }
    destroy_layers();
    free(composite_tile);
    free(mip_composite_tile);
    free(mip_source_tiles);
    free(background_tile);
    free(display_tile);
    free(display_lut);
    history_free();
    composite_tile = NULL;
    mip_composite_tile = NULL;
    mip_source_tiles = NULL;
    background_tile = NULL;
    display_tile = NULL;
    display_lut = NULL;
    display_lut_ready = 0;
    history_entry_count = 0;
    history_record_total = 0;
    history_cursor = 0;
    history_active = 0;
    if (brush) {
        mypaint_brush_unref(brush);
        brush = NULL;
    }
    tile_bytes = (size_t)MYPAINT_TILE_SIZE * (size_t)MYPAINT_TILE_SIZE * 4u * sizeof(uint16_t);
    composite_tile = (uint16_t *)calloc(1, tile_bytes);
    mip_composite_tile = (uint16_t *)calloc(1, tile_bytes);
    mip_source_tiles = (uint16_t *)calloc(WEB_MIP_MAX_SOURCES, tile_bytes);
    background_tile = (uint16_t *)calloc(1, tile_bytes);
    display_tile = (uint8_t *)calloc(1, (size_t)MYPAINT_TILE_SIZE * (size_t)MYPAINT_TILE_SIZE * 4u);
    display_lut = (uint8_t *)calloc((size_t)DISPLAY_LUT_VALUES * DISPLAY_LUT_NOISE, sizeof(uint8_t));
    display_lut_ready = 0;
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        group_tile[i] = (uint16_t *)calloc(1, tile_bytes);
        group_base_tile[i] = (uint16_t *)calloc(1, tile_bytes);
    }
    background_surface = NULL;
    layers[0] = web_surface_new(width, height);
    web_surface_set_write_callback(layers[0], history_capture_before);
    int group_storage_ok = 1;
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        if (!group_tile[i] || !group_base_tile[i]) group_storage_ok = 0;
    }
    if (!composite_tile || !mip_composite_tile || !mip_source_tiles || !background_tile || !display_tile || !display_lut || !layers[0] || !group_storage_ok) {
        free(composite_tile);
        free(mip_composite_tile);
        free(mip_source_tiles);
        free(background_tile);
        free(display_tile);
        free(display_lut);
        composite_tile = NULL;
        mip_composite_tile = NULL;
        mip_source_tiles = NULL;
        background_tile = NULL;
        display_tile = NULL;
        display_lut = NULL;
        display_lut_ready = 0;
        destroy_layers();
        return 0;
    }
    rebuild_display_lut();
    if (!history_ensure(1024)) {
        history_free();
    }
    layer_visible[0] = 1;
    paint_set_background_color(0xA8 / 255.0f, 0xA4 / 255.0f, 0x98 / 255.0f);
    layer_opacity[0] = 1.0f;
    layer_mode[0] = WEB_MODE_PIGMENT;
    layer_count = 1;
    active_layer = 0;
    node_append(0, -1);
    surface = layers[0];
    new_brush();
    dirty_roi.num_rectangles = 0;
    atomic_active = 0;
    suppress_atomic_end = 0;
    history_entry_count = 0;
    history_record_total = 0;
    history_cursor = 0;
    history_active = 0;
    history_active_start = 0;
    history_active_count = 0;
    paint_error_code = 0;
    return 1;
}

void begin_stroke(float x, float y, float xtilt, float ytilt,
                  float viewzoom, float viewrotation, float barrel_rotation)
{
    if (!brush || !surface) {
        return;
    }
    paint_history_begin();
    begin_atomic_internal();
    mypaint_brush_reset(brush);
    mypaint_brush_new_stroke(brush);
    /* Match MyPaint's abrupt Brushwork start. Prime the engine at the
     * contact point with zero pressure before the real input sample. */
    mypaint_brush_stroke_to(brush, surface_interface(), x, y, 0.0f,
                            xtilt, ytilt, 10.0, viewzoom, viewrotation,
                            barrel_rotation, 0);
    end_atomic_internal();
}

void set_brush_base_value(const char *setting_name, double base_value)
{
    if (!brush || !setting_name) {
        return;
    }
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    if (setting_id < MYPAINT_BRUSH_SETTINGS_COUNT) {
        mypaint_brush_set_base_value(brush, setting_id, (float)base_value);
    }
}

float get_brush_base_value(const char *setting_name)
{
    if (!brush || !setting_name) {
        return 0.0f;
    }
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    if (setting_id >= MYPAINT_BRUSH_SETTINGS_COUNT) {
        return 0.0f;
    }
    return mypaint_brush_get_base_value(brush, setting_id);
}

void set_brush_mapping_n(const char *setting_name, const char *input_name,
                         int number_of_mapping_points)
{
    if (!brush || !setting_name || !input_name) {
        return;
    }
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    MyPaintBrushInput input_id = mypaint_brush_input_from_cname(input_name);
    if (setting_id < MYPAINT_BRUSH_SETTINGS_COUNT && input_id < MYPAINT_BRUSH_INPUTS_COUNT) {
        mypaint_brush_set_mapping_n(brush, setting_id, input_id, number_of_mapping_points);
    }
}

void set_brush_mapping_point(const char *setting_name, const char *input_name,
                             int index, float x, float y)
{
    if (!brush || !setting_name || !input_name) {
        return;
    }
    MyPaintBrushSetting setting_id = mypaint_brush_setting_from_cname(setting_name);
    MyPaintBrushInput input_id = mypaint_brush_input_from_cname(input_name);
    if (setting_id < MYPAINT_BRUSH_SETTINGS_COUNT && input_id < MYPAINT_BRUSH_INPUTS_COUNT) {
        mypaint_brush_set_mapping_point(brush, setting_id, input_id, index, x, y);
    }
}

void reset_brush(void)
{
    if (brush) {
        mypaint_brush_reset(brush);
    }
}

void stroke_to(float x, float y, float pressure, float xtilt, float ytilt,
               double dtime, float viewzoom, float viewrotation,
               float barrel_rotation, int linear)
{
    if (!brush || !surface) {
        return;
    }
    begin_atomic_internal();
    mypaint_brush_stroke_to(brush, surface_interface(), x, y, pressure,
                            xtilt, ytilt, dtime, viewzoom, viewrotation,
                            barrel_rotation, linear ? 1 : 0);
    if (!suppress_atomic_end) {
        end_atomic_internal();
    }
}

void paint_begin_atomic(void)
{
    begin_atomic_internal();
}

int paint_end_atomic(void)
{
    return end_atomic_internal();
}

void paint_begin_batch(void)
{
    if (atomic_active) {
        end_atomic_internal();
    }
    suppress_atomic_end = 1;
    begin_atomic_internal();
}

int paint_end_batch(void)
{
    suppress_atomic_end = 0;
    return end_atomic_internal();
}

int paint_is_batch_done(void)
{
    /* Serial: the batch completes inside paint_end_batch(). */
    return 1;
}

int paint_end_batch_finish(void)
{
    /* Serial: no async finish step is needed. */
    return dirty_roi.num_rectangles;
}

int paint_get_width(void)
{
    return web_surface_get_width(surface);
}

int paint_get_height(void)
{
    return web_surface_get_height(surface);
}

int paint_get_error_code(void)
{
    return paint_error_code;
}

void paint_clear_error(void)
{
    paint_error_code = 0;
}

int paint_get_tiles_width(void)
{
    return web_surface_get_tiles_width(surface);
}

int paint_get_tiles_height(void)
{
    return web_surface_get_tiles_height(surface);
}

int paint_get_used_tile_count(void)
{
    return surface ? web_surface_get_used_tile_count(surface) : 0;
}

uintptr_t paint_get_tile_ptr(int tx, int ty)
{
    return (uintptr_t)web_surface_get_tile(surface, tx, ty);
}

static void render_node(int ref, int tx, int ty, uint16_t *target)
{
    if (WEB_REF_IS_GROUP(ref)) {
        const int group = WEB_REF_GROUP_ID(ref);
        if (group < 0 || group >= WEB_MAX_GROUPS || !group_alive[group] ||
            !group_visible[group] || !group_tile[group]) return;
        const int direct = group_pass_through[group] && !group_isolated[group] &&
                           group_mode[group] == WEB_MODE_NORMAL;
        uint16_t *content = group_tile[group];
        if (direct) {
            memcpy(group_base_tile[group], target, tile_bytes);
            for (int child = group_first_child[group]; child != WEB_REF_NONE;
                 child = node_next(child)) {
                render_node(child, tx, ty, target);
            }
            const uint32_t opacity = (uint32_t)(clamp01(group_opacity[group]) *
                                                32768.0f + 0.5f);
            if (opacity < 32768u) {
                const uint32_t inverse = 32768u - opacity;
                for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
                    for (int channel = 0; channel < 4; channel++) {
                        const uint32_t base = group_base_tile[group][pixel * 4 + channel];
                        const uint32_t result = target[pixel * 4 + channel];
                        target[pixel * 4 + channel] = (uint16_t)
                            ((base * inverse + result * opacity + 16384u) >> 15u);
                    }
                }
            }
            return;
        }
        memset(content, 0, tile_bytes);
        for (int child = group_first_child[group]; child != WEB_REF_NONE;
             child = node_next(child)) {
            render_node(child, tx, ty, content);
        }
        for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
            afterglow_layer_blend_over(&target[pixel * 4], &content[pixel * 4],
                                       group_opacity[group], group_mode[group]);
        }
        return;
    }
    if (ref < 0 || ref >= layer_count || !layer_visible[ref] || !layers[ref]) return;
    uint16_t *source_tile = web_surface_get_tile(layers[ref], tx, ty);
    if (!source_tile) return;
    for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
        afterglow_layer_blend_over(&target[pixel * 4], &source_tile[pixel * 4],
                                   layer_opacity[ref], layer_mode[ref]);
    }
}

uintptr_t paint_render_tile_ptr(int tx, int ty)
{
    if (!composite_tile) return 0;
    if (background_tile) memcpy(composite_tile, background_tile, tile_bytes);
    else for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
        composite_tile[pixel * 4] = background_color[0];
        composite_tile[pixel * 4 + 1] = background_color[1];
        composite_tile[pixel * 4 + 2] = background_color[2];
        composite_tile[pixel * 4 + 3] = background_color[3];
    }
    for (int child = root_first_child; child != WEB_REF_NONE; child = node_next(child)) {
        render_node(child, tx, ty, composite_tile);
    }
    return (uintptr_t)composite_tile;
}

static uint32_t display_noise(int pixel, int channel)
{
    uint32_t value = (uint32_t)(pixel * 747796405u + channel * 2891336453u + 12345u);
    value ^= value >> 16;
    return value & 255u;
}

static void rebuild_display_lut(void)
{
    if (!display_lut) return;
    const float inverse_eotf = 1.0f / display_eotf;
    for (uint32_t value = 0; value < DISPLAY_LUT_VALUES; value++) {
        for (uint32_t noise = 0; noise < DISPLAY_LUT_NOISE; noise++) {
            const float encoded = fminf(1.0f,
                (float)value / 32768.0f + (float)noise / (255.0f * 32768.0f));
            display_lut[(size_t)value * DISPLAY_LUT_NOISE + noise] =
                (uint8_t)(powf(encoded, inverse_eotf) * 255.0f + 0.5f);
        }
    }
    display_lut_ready = 1;
}

void paint_set_eotf(float eotf)
{
    if (isfinite(eotf) && eotf > 0.0f) {
        if (display_lut_ready && fabsf(display_eotf - eotf) < 0.0001f) return;
        display_eotf = eotf;
        rebuild_display_lut();
    }
}

static uintptr_t render_display_tile(const uint16_t *source)
{
    if (!source || !display_tile || !display_lut) return 0;
    for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
        const uint32_t alpha = source[pixel * 4 + 3];
        uint32_t r = 0;
        uint32_t g = 0;
        uint32_t b = 0;
        if (alpha != 0) {
            const uint32_t round_alpha = alpha / 2u;
            r = ((uint32_t)source[pixel * 4] << 15u) + round_alpha;
            g = ((uint32_t)source[pixel * 4 + 1] << 15u) + round_alpha;
            b = ((uint32_t)source[pixel * 4 + 2] << 15u) + round_alpha;
            r /= alpha;
            g /= alpha;
            b /= alpha;
            if (r > 32768u) r = 32768u;
            if (g > 32768u) g = 32768u;
            if (b > 32768u) b = 32768u;
        }
        display_tile[pixel * 4] = display_lut[(size_t)r * DISPLAY_LUT_NOISE + display_noise(pixel, 0)];
        display_tile[pixel * 4 + 1] = display_lut[(size_t)g * DISPLAY_LUT_NOISE + display_noise(pixel, 1)];
        display_tile[pixel * 4 + 2] = display_lut[(size_t)b * DISPLAY_LUT_NOISE + display_noise(pixel, 2)];
        display_tile[pixel * 4 + 3] = (uint8_t)((alpha * 255u + 16384u) / 32768u);
    }
    return (uintptr_t)display_tile;
}

uintptr_t paint_render_rgba8_tile_ptr(int tx, int ty)
{
    return render_display_tile((const uint16_t *)paint_render_tile_ptr(tx, ty));
}

uintptr_t paint_render_layer_rgba8_tile_ptr(int layer_id, int tx, int ty)
{
    if (!composite_tile || layer_id < 0 || layer_id >= layer_count || !layers[layer_id]) return 0;
    uint16_t *source = web_surface_get_tile(layers[layer_id], tx, ty);
    if (!source) {
        memset(composite_tile, 0, tile_bytes);
        source = composite_tile;
    }
    return render_display_tile(source);
}

int paint_write_rgba8_tile(int tx, int ty, const uint8_t *source)
{
    if (!surface || !source || tx < 0 || ty < 0 ||
        tx >= web_surface_get_tiles_width(surface) ||
        ty >= web_surface_get_tiles_height(surface)) return 0;
    uint16_t *tile = web_surface_get_or_create_tile(surface, tx, ty);
    if (!tile) return 0;
    for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
        const uint32_t alpha = ((uint32_t)source[pixel * 4 + 3] * 32768u + 127u) / 255u;
        for (int channel = 0; channel < 3; channel++) {
            const float encoded = (float)source[pixel * 4 + channel] / 255.0f;
            const uint32_t linear = (uint32_t)(powf(encoded, display_eotf) * 32768.0f + 0.5f);
            tile[pixel * 4 + channel] = (uint16_t)((linear * alpha + 16384u) >> 15u);
        }
        tile[pixel * 4 + 3] = (uint16_t)alpha;
    }
    mark_full_dirty();
    return 1;
}

int paint_region_has_paint(int tx, int ty, int level)
{
    if (level <= 0) {
        for (int layer = 0; layer < layer_count; layer++) {
            if (layer_visible[layer] && layers[layer] &&
                web_surface_get_tile(layers[layer], tx, ty)) {
                return 1;
            }
        }
        return 0;
    }
    if (level > 2) level = 2;
    const int scale = 1 << level;
    for (int layer = 0; layer < layer_count; layer++) {
        if (!layer_visible[layer] || !layers[layer]) continue;
        for (int sy = 0; sy < scale; sy++) {
            for (int sx = 0; sx < scale; sx++) {
                if (web_surface_get_tile(layers[layer],
                        tx * scale + sx, ty * scale + sy)) {
                    return 1;
                }
            }
        }
    }
    return 0;
}

uintptr_t paint_render_rgba8_mip_tile_ptr(int tx, int ty, int level)
{
    if (!mip_composite_tile || !mip_source_tiles || level <= 0) {
        return paint_render_rgba8_tile_ptr(tx, ty);
    }
    if (level > 2) level = 2;
    const int scale = 1 << level;
    int has_paint_tiles = 0;
    for (int layer = 0; layer < layer_count; layer++) {
        if (layers[layer] && web_surface_get_used_tile_count(layers[layer]) > 0) {
            has_paint_tiles = 1;
            break;
        }
    }
    if (!has_paint_tiles) {
        for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
            for (int channel = 0; channel < 4; channel++) {
                mip_composite_tile[pixel * 4 + channel] = background_color[channel];
            }
        }
        return render_display_tile(mip_composite_tile);
    }
    int region_has_paint = 0;
    for (int layer = 0; layer < layer_count && !region_has_paint; layer++) {
        if (!layer_visible[layer] || !layers[layer]) continue;
        for (int source_tile_y = 0; source_tile_y < scale && !region_has_paint; source_tile_y++) {
            for (int source_tile_x = 0; source_tile_x < scale && !region_has_paint; source_tile_x++) {
                if (web_surface_get_tile(layers[layer],
                        tx * scale + source_tile_x, ty * scale + source_tile_y)) {
                    region_has_paint = 1;
                }
            }
        }
    }
    if (!region_has_paint) {
        for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
            for (int channel = 0; channel < 4; channel++) {
                mip_composite_tile[pixel * 4 + channel] = background_color[channel];
            }
        }
        return render_display_tile(mip_composite_tile);
    }
    const size_t tile_pixels = (size_t)MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE * 4u;
    for (int source_tile_y = 0; source_tile_y < scale; source_tile_y++) {
        for (int source_tile_x = 0; source_tile_x < scale; source_tile_x++) {
            const int source_tx = tx * scale + source_tile_x;
            const int source_ty = ty * scale + source_tile_y;
            const int source_index = source_tile_y * scale + source_tile_x;
            const uintptr_t source_address = paint_render_tile_ptr(source_tx, source_ty);
            if (source_address) {
                memcpy(mip_source_tiles + (size_t)source_index * tile_pixels,
                       (const void *)source_address, tile_bytes);
            } else {
                memset(mip_source_tiles + (size_t)source_index * tile_pixels, 0, tile_bytes);
            }
        }
    }
    for (int pixel_y = 0; pixel_y < MYPAINT_TILE_SIZE; pixel_y++) {
        for (int pixel_x = 0; pixel_x < MYPAINT_TILE_SIZE; pixel_x++) {
            uint64_t sum[4] = {0, 0, 0, 0};
            for (int sample_y = 0; sample_y < scale; sample_y++) {
                const int source_y = pixel_y * scale + sample_y;
                const int source_tile_y = source_y / MYPAINT_TILE_SIZE;
                const int local_y = source_y % MYPAINT_TILE_SIZE;
                for (int sample_x = 0; sample_x < scale; sample_x++) {
                    const int source_x = pixel_x * scale + sample_x;
                    const int source_tile_x = source_x / MYPAINT_TILE_SIZE;
                    const int local_x = source_x % MYPAINT_TILE_SIZE;
                    const int source_index = source_tile_y * scale + source_tile_x;
                    const uint16_t *source = mip_source_tiles +
                        (size_t)source_index * tile_pixels;
                    const int offset = (local_y * MYPAINT_TILE_SIZE + local_x) * 4;
                    for (int channel = 0; channel < 4; channel++) {
                        sum[channel] += source[offset + channel];
                    }
                }
            }
            const uint32_t sample_count = (uint32_t)(scale * scale);
            for (int channel = 0; channel < 4; channel++) {
                mip_composite_tile[(pixel_y * MYPAINT_TILE_SIZE + pixel_x) * 4 + channel] =
                    (uint16_t)((sum[channel] + sample_count / 2u) / sample_count);
            }
        }
    }
    return render_display_tile(mip_composite_tile);
}

int paint_get_dirty_count(void)
{
    return dirty_roi.num_rectangles;
}

void paint_get_dirty_rect(int index, int *out_rect)
{
    if (!out_rect || index < 0 || index >= dirty_roi.num_rectangles) {
        return;
    }
    out_rect[0] = dirty_rects[index].x;
    out_rect[1] = dirty_rects[index].y;
    out_rect[2] = dirty_rects[index].width;
    out_rect[3] = dirty_rects[index].height;
}

void paint_clear_dirty(void)
{
    dirty_roi.num_rectangles = 0;
}

int history_ensure(int needed)
{
    if (needed <= history_capacity) return 1;
    int new_cap = history_capacity > 0 ? history_capacity : 1024;
    while (new_cap < needed) new_cap *= 2;
    const size_t old_bytes = (size_t)history_capacity * tile_bytes;
    const size_t new_bytes = (size_t)new_cap * tile_bytes;
    uint16_t *nb = (uint16_t *)realloc(history_before, new_bytes);
    if (!nb) return 0;
    history_before = nb;
    uint16_t *na = (uint16_t *)realloc(history_after, new_bytes);
    if (!na) return 0;
    history_after = na;
    int *ntx = (int *)realloc(history_tx, (size_t)new_cap * sizeof(int));
    if (!ntx) return 0;
    history_tx = ntx;
    int *nty = (int *)realloc(history_ty, (size_t)new_cap * sizeof(int));
    if (!nty) return 0;
    history_ty = nty;
    if (new_bytes > old_bytes) {
        memset((uint8_t *)history_before + old_bytes, 0, new_bytes - old_bytes);
        memset((uint8_t *)history_after + old_bytes, 0, new_bytes - old_bytes);
    }
    history_capacity = new_cap;
    return 1;
}

void history_free(void)
{
    free(history_before);
    free(history_after);
    free(history_tx);
    free(history_ty);
    history_before = NULL;
    history_after = NULL;
    history_tx = NULL;
    history_ty = NULL;
    history_capacity = 0;
    history_entry_count = 0;
}

static int history_find_entry(int start, int count, int tx, int ty)
{
    for (int i = 0; i < count; i++) {
        const int entry = start + i;
        if (history_tx[entry] == tx && history_ty[entry] == ty) return entry;
    }
    return -1;
}

static volatile int history_spinlock = 0;

static void history_capture_before(WebPaintSurface *owner, int tx, int ty,
                                   uint16_t *tile)
{
    if (!history_active || !tile || history_active_layer < 0 ||
        history_active_layer >= layer_count || owner != layers[history_active_layer]) {
        return;
    }
    /* Spinlock — no Atomics.wait, can't deadlock with pthread_join */
    while (__atomic_test_and_set(&history_spinlock, __ATOMIC_ACQUIRE)) {}
    if (history_find_entry(history_active_start, history_active_count, tx, ty) >= 0) {
        __atomic_clear(&history_spinlock, __ATOMIC_RELEASE);
        return;
    }
    if (history_entry_count + 1 > history_capacity) {
        /* Don't realloc from a pthread worker — it deadlocks on wasmMemory.grow. */
        __atomic_clear(&history_spinlock, __ATOMIC_RELEASE);
        return;
    }
    if (!history_ensure(history_entry_count + 1)) {
        paint_error_code = 2;
        history_active = 0;
        history_entry_count = history_active_start;
        __atomic_clear(&history_spinlock, __ATOMIC_RELEASE);
        return;
    }
    const int entry = history_entry_count++;
    history_tx[entry] = tx;
    history_ty[entry] = ty;
    memcpy(history_before + (size_t)entry * tile_bytes / sizeof(uint16_t),
           tile, tile_bytes);
    memset(history_after + (size_t)entry * tile_bytes / sizeof(uint16_t),
           0, tile_bytes);
    history_active_count++;
    __atomic_clear(&history_spinlock, __ATOMIC_RELEASE);
}

static void history_drop_oldest(void)
{
    if (history_record_total <= 0) return;
    const int removed = history_record_count[0];
    const int remaining = history_entry_count - removed;
    if (remaining > 0) {
        memmove(history_before, history_before + (size_t)removed * tile_bytes,
                (size_t)remaining * tile_bytes);
        memmove(history_after, history_after + (size_t)removed * tile_bytes,
                (size_t)remaining * tile_bytes);
        memmove(history_tx, history_tx + removed, (size_t)remaining * sizeof(int));
        memmove(history_ty, history_ty + removed, (size_t)remaining * sizeof(int));
    }
    for (int i = 1; i < history_record_total; i++) {
        history_record_start[i - 1] = history_record_start[i] - removed;
        history_record_count[i - 1] = history_record_count[i];
        history_record_layer[i - 1] = history_record_layer[i];
    }
    history_record_total--;
    if (history_cursor > 0) history_cursor--;
    history_entry_count = remaining;
}

void paint_history_begin(void)
{
    if (!surface || !history_before || !history_after || history_active) return;
    if (history_cursor < history_record_total) {
        history_record_total = history_cursor;
        history_entry_count = history_cursor > 0
            ? history_record_start[history_cursor - 1] +
              history_record_count[history_cursor - 1] : 0;
    }
    while (history_record_total >= WEB_HISTORY_RECORDS) history_drop_oldest();
    history_active_start = history_entry_count;
    history_active_count = 0;
    history_active_layer = active_layer;
    history_active = 1;
}

void paint_history_commit(void)
{
    if (!surface || !history_after || !history_active) return;
    if (history_active_layer != active_layer) {
        history_active = 0;
        history_entry_count = history_active_start;
        return;
    }
    for (int i = 0; i < history_active_count; i++) {
        const int entry = history_active_start + i;
        uint16_t *after = history_after + (size_t)entry * tile_bytes / sizeof(uint16_t);
        uint16_t *tile = web_surface_get_tile(
            surface, history_tx[entry], history_ty[entry]);
        if (tile) memcpy(after, tile, tile_bytes);
    }
    if (history_active_count > 0) {
        history_record_start[history_record_total] = history_active_start;
        history_record_count[history_record_total] = history_active_count;
        history_record_layer[history_record_total] = history_active_layer;
        history_record_total++;
        history_cursor = history_record_total;
    } else {
        history_entry_count = history_active_start;
    }
    history_active = 0;
}

static int history_restore_record(int record, int redo)
{
    if (record < 0 || record >= history_record_total) return 0;
    const int layer = history_record_layer[record];
    if (layer < 0 || layer >= layer_count || !layers[layer]) return 0;
    active_layer = layer;
    surface = layers[layer];
    const int start = history_record_start[record];
    const int count = history_record_count[record];
    for (int i = 0; i < count; i++) {
        const int entry = start + i;
        uint16_t *tile = web_surface_get_or_create_tile(
            surface, history_tx[entry], history_ty[entry]);
        if (!tile) {
            paint_error_code = 1;
            return 0;
        }
        const uint16_t *source = (redo ? history_after : history_before) +
            (size_t)entry * tile_bytes / sizeof(uint16_t);
        memcpy(tile, source, tile_bytes);
    }
    mark_full_dirty();
    return 1;
}

int paint_history_undo(void)
{
    if (history_cursor <= 0) return 0;
    if (!history_restore_record(history_cursor - 1, 0)) return 0;
    history_cursor--;
    return 1;
}

int paint_history_redo(void)
{
    if (history_cursor >= history_record_total) return 0;
    if (!history_restore_record(history_cursor, 1)) return 0;
    history_cursor++;
    return 1;
}

int paint_history_can_undo(void)
{
    return history_cursor > 0;
}

int paint_history_can_redo(void)
{
    return history_cursor < history_record_total;
}

void paint_set_background_color(float r, float g, float b)
{
    const float er = powf(clamp01(r), 2.2f);
    const float eg = powf(clamp01(g), 2.2f);
    const float eb = powf(clamp01(b), 2.2f);
    background_color[0] = (uint16_t)(er * 32768.0f + 0.5f);
    background_color[1] = (uint16_t)(eg * 32768.0f + 0.5f);
    background_color[2] = (uint16_t)(eb * 32768.0f + 0.5f);
    background_color[3] = 32768;
    if (background_tile) {
        for (int pixel = 0; pixel < MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE; pixel++) {
            memcpy(background_tile + pixel * 4, background_color, sizeof(background_color));
        }
    }
    mark_full_dirty();
}

void paint_clear(void)
{
    if (!surface) {
        return;
    }
    web_surface_clear(surface);
    mark_full_dirty();
}

void paint_clear_background(void)
{
    memset(background_color, 0, sizeof(background_color));
    if (background_tile) memset(background_tile, 0, tile_bytes);
    mark_full_dirty();
}

void paint_pick_color(float x, float y, float radius, float paint,
                      float *out_rgba)
{
    if (!out_rgba || !surface) {
        return;
    }
    mypaint_surface_get_color(surface_interface(), x, y, radius,
                              &out_rgba[0], &out_rgba[1], &out_rgba[2],
                              &out_rgba[3], paint);
}

void paint_set_symmetry(int active, float center_x, float center_y,
                        float angle, int symmetry_type, int lines)
{
    if (!surface) {
        return;
    }
    web_surface_set_symmetry(surface, active, center_x, center_y, angle,
                              symmetry_type, lines);
}

int paint_get_layer_count(void)
{
    return layer_count;
}

int paint_get_active_layer(void)
{
    return active_layer;
}

int paint_set_active_layer(int layer_id)
{
    if (layer_id < 0 || layer_id >= layer_count || !layers[layer_id]) {
        return 0;
    }
    active_layer = layer_id;
    surface = layers[layer_id];
    dirty_roi.num_rectangles = 0;
    return 1;
}

static int remap_layer_ref(int ref, int removed)
{
    if (ref == WEB_REF_NONE || ref == removed) return WEB_REF_NONE;
    if (ref > removed) return ref - 1;
    return ref;
}

static void history_reset_all(void)
{
    history_entry_count = 0;
    history_record_total = 0;
    history_cursor = 0;
    history_active = 0;
    history_active_start = 0;
    history_active_count = 0;
}

int paint_create_layer(void)
{
    if (layer_count >= WEB_MAX_LAYERS || !surface) return -1;
    WebPaintSurface *new_surface = web_surface_new(
        web_surface_get_width(surface), web_surface_get_height(surface));
    if (!new_surface) return -1;
    web_surface_set_write_callback(new_surface, history_capture_before);
    const int id = layer_count++;
    layers[id] = new_surface;
    layer_visible[id] = 1;
    layer_opacity[id] = 1.0f;
    layer_mode[id] = WEB_MODE_PIGMENT;
    node_append(id, -1);
    active_layer = id;
    surface = new_surface;
    mark_full_dirty();
    return id;
}

int paint_delete_layer(int layer_id)
{
    if (layer_count <= 1 || layer_id < 0 || layer_id >= layer_count) return 0;
    node_remove(layer_id);
    web_surface_destroy(layers[layer_id]);
    for (int i = layer_id; i + 1 < layer_count; i++) {
        layers[i] = layers[i + 1];
        layer_visible[i] = layer_visible[i + 1];
        layer_opacity[i] = layer_opacity[i + 1];
        layer_mode[i] = layer_mode[i + 1];
        layer_parent[i] = layer_parent[i + 1];
        layer_next[i] = layer_next[i + 1];
        layer_previous[i] = layer_previous[i + 1];
    }
    layer_count--;
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        group_first_child[i] = remap_layer_ref(group_first_child[i], layer_id);
        group_last_child[i] = remap_layer_ref(group_last_child[i], layer_id);
        group_next[i] = remap_layer_ref(group_next[i], layer_id);
        group_previous[i] = remap_layer_ref(group_previous[i], layer_id);
    }
    root_first_child = remap_layer_ref(root_first_child, layer_id);
    root_last_child = remap_layer_ref(root_last_child, layer_id);
    for (int i = 0; i < layer_count; i++) {
        layer_next[i] = remap_layer_ref(layer_next[i], layer_id);
        layer_previous[i] = remap_layer_ref(layer_previous[i], layer_id);
    }
    layers[layer_count] = NULL;
    layer_visible[layer_count] = 0;
    layer_opacity[layer_count] = 0.0f;
    layer_mode[layer_count] = WEB_MODE_NORMAL;
    layer_parent[layer_count] = -2;
    layer_next[layer_count] = WEB_REF_NONE;
    layer_previous[layer_count] = WEB_REF_NONE;
    if (active_layer == layer_id) active_layer = layer_id < layer_count ? layer_id : layer_count - 1;
    else if (active_layer > layer_id) active_layer--;
    surface = layers[active_layer];
    history_reset_all();
    mark_full_dirty();
    return 1;
}

int paint_get_layer_visible(int layer_id)
{
    if (layer_id < 0 || layer_id >= layer_count) return 0;
    return layer_visible[layer_id] ? 1 : 0;
}

void paint_set_layer_visible(int layer_id, int visible)
{
    if (layer_id < 0 || layer_id >= layer_count) return;
    layer_visible[layer_id] = visible ? 1 : 0;
    mark_full_dirty();
}

void paint_set_layer_opacity(int layer_id, float opacity)
{
    if (layer_id < 0 || layer_id >= layer_count) return;
    if (opacity < 0) opacity = 0;
    if (opacity > 1) opacity = 1;
    layer_opacity[layer_id] = opacity;
    mark_full_dirty();
}

float paint_get_layer_opacity(int layer_id)
{
    if (layer_id < 0 || layer_id >= layer_count) return 0.0f;
    return layer_opacity[layer_id];
}

int paint_get_layer_mode(int layer_id)
{
    if (layer_id < 0 || layer_id >= layer_count) return WEB_MODE_NORMAL;
    return layer_mode[layer_id];
}

void paint_set_layer_mode(int layer_id, int mode)
{
    if (layer_id < 0 || layer_id >= layer_count) return;
    if (mode < 0 || mode >= WEB_MODE_COUNT) return;
    layer_mode[layer_id] = mode;
    mark_full_dirty();
}

int paint_get_layer_group(int layer_id)
{
    if (layer_id < 0 || layer_id >= layer_count) return -1;
    return layer_parent[layer_id];
}

int paint_set_layer_group(int layer_id, int group_id)
{
    if (layer_id < 0 || layer_id >= layer_count) return 0;
    if (group_id >= 0 && (group_id >= WEB_MAX_GROUPS || !group_alive[group_id])) return 0;
    node_remove(layer_id);
    node_append(layer_id, group_id);
    history_reset_all();
    mark_full_dirty();
    return 1;
}

static int paint_move_node(int ref, int direction)
{
    const int neighbor = direction < 0 ? node_previous(ref) : node_next(ref);
    if (neighbor == WEB_REF_NONE) return 0;
    const int parent = node_parent(ref);
    const int after = direction < 0 ? neighbor : node_next(neighbor);
    node_remove(ref);
    if (direction < 0) node_insert_before(ref, neighbor);
    else if (after == WEB_REF_NONE) node_append(ref, parent);
    else node_insert_before(ref, after);
    mark_full_dirty();
    return 1;
}

int paint_move_layer(int layer_id, int direction)
{
    if (layer_id < 0 || layer_id >= layer_count || (direction != -1 && direction != 1)) return 0;
    return paint_move_node(layer_id, direction);
}

int paint_get_group_count(void)
{
    return group_count;
}

int paint_get_group_alive(int group_id)
{
    return group_id >= 0 && group_id < WEB_MAX_GROUPS && group_alive[group_id] ? 1 : 0;
}

int paint_get_group_parent(int group_id)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return -1;
    return group_parent[group_id];
}

int paint_create_group(void)
{
    int group = -1;
    for (int i = 0; i < WEB_MAX_GROUPS; i++) {
        if (!group_alive[i]) {
            group = i;
            break;
        }
    }
    if (group < 0) return -1;
    group_alive[group] = 1;
    group_visible[group] = 1;
    group_pass_through[group] = 0;
    group_isolated[group] = 1;
    group_opacity[group] = 1.0f;
    group_mode[group] = WEB_MODE_NORMAL;
    group_first_child[group] = WEB_REF_NONE;
    group_last_child[group] = WEB_REF_NONE;
    group_count = group + 1 > group_count ? group + 1 : group_count;
    node_append(WEB_REF_GROUP(group), -1);
    mark_full_dirty();
    return group;
}

int paint_delete_group(int group_id)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return 0;
    const int parent = group_parent[group_id];
    int child = group_first_child[group_id];
    while (child != WEB_REF_NONE) {
        const int next = node_next(child);
        node_remove(child);
        node_append(child, parent);
        child = next;
    }
    node_remove(WEB_REF_GROUP(group_id));
    group_alive[group_id] = 0;
    group_visible[group_id] = 0;
    group_first_child[group_id] = WEB_REF_NONE;
    group_last_child[group_id] = WEB_REF_NONE;
    group_parent[group_id] = -2;
    while (group_count > 0 && !group_alive[group_count - 1]) group_count--;
    history_reset_all();
    mark_full_dirty();
    return 1;
}

int paint_set_group_parent(int group_id, int parent_id)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return 0;
    if (parent_id >= 0 && (parent_id >= WEB_MAX_GROUPS || !group_alive[parent_id])) return 0;
    if (parent_id == group_id || (parent_id >= 0 && group_contains(group_id, parent_id))) return 0;
    node_remove(WEB_REF_GROUP(group_id));
    node_append(WEB_REF_GROUP(group_id), parent_id);
    history_reset_all();
    mark_full_dirty();
    return 1;
}

int paint_get_group_visible(int group_id)
{
    return group_id >= 0 && group_id < WEB_MAX_GROUPS && group_alive[group_id] && group_visible[group_id];
}

void paint_set_group_visible(int group_id, int visible)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return;
    group_visible[group_id] = visible ? 1 : 0;
    mark_full_dirty();
}

float paint_get_group_opacity(int group_id)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return 0.0f;
    return group_opacity[group_id];
}

void paint_set_group_opacity(int group_id, float opacity)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return;
    group_opacity[group_id] = clamp01(opacity);
    mark_full_dirty();
}

int paint_get_group_mode(int group_id)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return WEB_MODE_NORMAL;
    return group_mode[group_id];
}

void paint_set_group_mode(int group_id, int mode)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id] ||
        mode < 0 || mode >= WEB_MODE_COUNT) return;
    group_mode[group_id] = mode;
    mark_full_dirty();
}

int paint_get_group_pass_through(int group_id)
{
    return group_id >= 0 && group_id < WEB_MAX_GROUPS && group_alive[group_id] && group_pass_through[group_id];
}

void paint_set_group_pass_through(int group_id, int value)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return;
    group_pass_through[group_id] = value ? 1 : 0;
    mark_full_dirty();
}

int paint_get_group_isolated(int group_id)
{
    return group_id >= 0 && group_id < WEB_MAX_GROUPS && group_alive[group_id] && group_isolated[group_id];
}

void paint_set_group_isolated(int group_id, int value)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id]) return;
    group_isolated[group_id] = value ? 1 : 0;
    mark_full_dirty();
}

int paint_move_group(int group_id, int direction)
{
    if (group_id < 0 || group_id >= WEB_MAX_GROUPS || !group_alive[group_id] ||
        (direction != -1 && direction != 1)) return 0;
    return paint_move_node(WEB_REF_GROUP(group_id), direction);
}

void paint_destroy(void)
{
    if (atomic_active) {
        end_atomic_internal();
    }
    if (brush) {
        mypaint_brush_unref(brush);
        brush = NULL;
    }
    destroy_layers();
    free(composite_tile);
    free(mip_composite_tile);
    free(mip_source_tiles);
    free(background_tile);
    free(display_tile);
    free(display_lut);
    history_free();
    composite_tile = NULL;
    mip_composite_tile = NULL;
    mip_source_tiles = NULL;
    background_tile = NULL;
    display_tile = NULL;
    display_lut = NULL;
    display_lut_ready = 0;
    tile_bytes = 0;
}
