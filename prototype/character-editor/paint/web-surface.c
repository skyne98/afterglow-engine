#include "web-surface.h"

#include <stdlib.h>
#include <string.h>

#define WEB_SURFACE_MIN_HASH_SIZE 8192

struct WebPaintSurface {
    MyPaintTiledSurface parent;
    uint16_t **tiles;
    uint16_t *null_tile;
    size_t tile_bytes;
    int tile_capacity;
    int tile_count;
    int capacity_failed;
    WebSurfaceWriteCallback write_callback;
    int hash_size;
    uint8_t *tile_used;
    int32_t *used_tile_x;
    int32_t *used_tile_y;
    int32_t *tile_x;
    int32_t *tile_y;
    int32_t *tile_slot;
    int tiles_width;
    int tiles_height;
    int width;
    int height;
};

static uint32_t tile_hash(int x, int y)
{
    uint32_t value = (uint32_t)x * 0x9E3779B1u;
    value ^= (uint32_t)y * 0x85EBCA77u;
    value ^= value >> 16;
    value *= 0xC2B2AE3Du;
    return value ^ (value >> 13);
}

static int find_tile_slot(const WebPaintSurface *surface, int tx, int ty)
{
    const uint32_t start = tile_hash(tx, ty) & (uint32_t)(surface->hash_size - 1);
    for (uint32_t probe = 0; probe < (uint32_t)surface->hash_size; probe++) {
        const uint32_t slot = (start + probe) & (uint32_t)(surface->hash_size - 1);
        if (!surface->tile_used[slot]) return -1;
        if (surface->tile_x[slot] == tx && surface->tile_y[slot] == ty) {
            return surface->tile_slot[slot];
        }
    }
    return -1;
}

static int create_tile_slot(WebPaintSurface *surface, int tx, int ty)
{
    const uint32_t start = tile_hash(tx, ty) & (uint32_t)(surface->hash_size - 1);
    for (uint32_t probe = 0; probe < (uint32_t)surface->hash_size; probe++) {
        const uint32_t slot = (start + probe) & (uint32_t)(surface->hash_size - 1);
        if (surface->tile_used[slot]) {
            if (surface->tile_x[slot] == tx && surface->tile_y[slot] == ty) {
                return surface->tile_slot[slot];
            }
            continue;
        }
        if (surface->tile_count >= surface->tile_capacity) {
            surface->capacity_failed = 1;
            return -1;
        }
        uint16_t *tile = (uint16_t *)calloc(1, surface->tile_bytes);
        if (!tile) {
            surface->capacity_failed = 1;
            return -1;
        }
        const int tile_slot = surface->tile_count++;
        surface->tiles[tile_slot] = tile;
        surface->tile_used[slot] = 1;
        surface->used_tile_x[tile_slot] = tx;
        surface->used_tile_y[tile_slot] = ty;
        surface->tile_x[slot] = tx;
        surface->tile_y[slot] = ty;
        surface->tile_slot[slot] = tile_slot;
        return tile_slot;
    }
    surface->capacity_failed = 1;
    return -1;
}

static uint16_t *tile_at_slot(WebPaintSurface *surface, int tile_slot)
{
    if (!surface || tile_slot < 0 || tile_slot >= surface->tile_count) return NULL;
    return surface->tiles[tile_slot];
}

static void reset_null_tile(WebPaintSurface *surface)
{
    memset(surface->null_tile, 0, surface->tile_bytes);
}

static void tile_request_start(MyPaintTiledSurface *tiled_surface,
                               MyPaintTileRequest *request)
{
    WebPaintSurface *surface = (WebPaintSurface *)tiled_surface;
    const int tx = request->tx;
    const int ty = request->ty;
    if (request->mipmap_level != 0) {
        request->buffer = surface->null_tile;
        request->context = (gpointer)surface->null_tile;
        return;
    }

    int tile_slot = find_tile_slot(surface, tx, ty);
    if (tile_slot < 0 && !request->readonly) {
        tile_slot = create_tile_slot(surface, tx, ty);
    }
    if (tile_slot < 0) {
        request->buffer = surface->null_tile;
        request->context = (gpointer)surface->null_tile;
        return;
    }
    request->buffer = tile_at_slot(surface, tile_slot);
    request->context = NULL;
    if (!request->readonly && surface->write_callback) {
        surface->write_callback(surface, tx, ty, request->buffer);
    }
}

static void tile_request_end(MyPaintTiledSurface *tiled_surface,
                             MyPaintTileRequest *request)
{
    WebPaintSurface *surface = (WebPaintSurface *)tiled_surface;
    if (request->context == (gpointer)surface->null_tile) {
        reset_null_tile(surface);
    }
}

static void free_tiles(WebPaintSurface *surface)
{
    if (!surface) return;
    for (int i = 0; i < surface->tile_count; i++) {
        free(surface->tiles[i]);
        surface->tiles[i] = NULL;
    }
}

static void web_surface_free(MyPaintSurface *base)
{
    WebPaintSurface *surface = (WebPaintSurface *)base;
    free_tiles(surface);
    mypaint_tiled_surface_destroy(&surface->parent);
    free(surface->tiles);
    free(surface->tile_used);
    free(surface->used_tile_x);
    free(surface->used_tile_y);
    free(surface->tile_x);
    free(surface->tile_y);
    free(surface->tile_slot);
    free(surface->null_tile);
    free(surface);
}

WebPaintSurface *web_surface_new(int width, int height)
{
    if (width <= 0 || height <= 0) return NULL;

    WebPaintSurface *surface = (WebPaintSurface *)calloc(1, sizeof(WebPaintSurface));
    if (!surface) return NULL;

    mypaint_tiled_surface_init(&surface->parent, tile_request_start, tile_request_end);
    surface->parent.parent.destroy = web_surface_free;
    surface->parent.threadsafe_tile_requests = FALSE;
    surface->width = width;
    surface->height = height;
    surface->tiles_width = (width + MYPAINT_TILE_SIZE - 1) / MYPAINT_TILE_SIZE;
    surface->tiles_height = (height + MYPAINT_TILE_SIZE - 1) / MYPAINT_TILE_SIZE;
    surface->tile_bytes = (size_t)MYPAINT_TILE_SIZE * (size_t)MYPAINT_TILE_SIZE *
                          4u * sizeof(uint16_t);
    const size_t document_tiles = (size_t)surface->tiles_width *
                                  (size_t)surface->tiles_height;
    surface->tile_capacity = document_tiles < 1 ? 1 : (int)document_tiles;

    int hash_size = WEB_SURFACE_MIN_HASH_SIZE;
    while ((size_t)hash_size < (size_t)surface->tile_capacity * 2u) {
        hash_size <<= 1;
    }
    surface->hash_size = hash_size;

    surface->tiles = (uint16_t **)calloc((size_t)surface->tile_capacity,
                                         sizeof(uint16_t *));
    surface->tile_used = (uint8_t *)calloc((size_t)hash_size, sizeof(uint8_t));
    surface->used_tile_x = (int32_t *)calloc((size_t)surface->tile_capacity,
                                             sizeof(int32_t));
    surface->used_tile_y = (int32_t *)calloc((size_t)surface->tile_capacity,
                                             sizeof(int32_t));
    surface->tile_x = (int32_t *)calloc((size_t)hash_size, sizeof(int32_t));
    surface->tile_y = (int32_t *)calloc((size_t)hash_size, sizeof(int32_t));
    surface->tile_slot = (int32_t *)calloc((size_t)hash_size, sizeof(int32_t));
    surface->null_tile = (uint16_t *)calloc(1, surface->tile_bytes);
    if (!surface->tiles || !surface->tile_used || !surface->used_tile_x ||
        !surface->used_tile_y || !surface->tile_x || !surface->tile_y ||
        !surface->tile_slot || !surface->null_tile) {
        web_surface_free((MyPaintSurface *)surface);
        return NULL;
    }
    return surface;
}

void web_surface_destroy(WebPaintSurface *surface)
{
    if (surface) web_surface_free((MyPaintSurface *)surface);
}

MyPaintSurface *web_surface_interface(WebPaintSurface *surface)
{
    return surface ? (MyPaintSurface *)surface : NULL;
}

int web_surface_get_width(const WebPaintSurface *surface)
{
    return surface ? surface->width : 0;
}

int web_surface_get_height(const WebPaintSurface *surface)
{
    return surface ? surface->height : 0;
}

int web_surface_get_tiles_width(const WebPaintSurface *surface)
{
    return surface ? surface->tiles_width : 0;
}

int web_surface_get_tiles_height(const WebPaintSurface *surface)
{
    return surface ? surface->tiles_height : 0;
}

uint16_t *web_surface_get_tile(WebPaintSurface *surface, int tx, int ty)
{
    if (!surface) return NULL;
    return tile_at_slot(surface, find_tile_slot(surface, tx, ty));
}

uint16_t *web_surface_get_or_create_tile(WebPaintSurface *surface, int tx, int ty)
{
    if (!surface) return NULL;
    int tile_slot = find_tile_slot(surface, tx, ty);
    if (tile_slot < 0) tile_slot = create_tile_slot(surface, tx, ty);
    return tile_at_slot(surface, tile_slot);
}

int web_surface_take_capacity_error(WebPaintSurface *surface)
{
    if (!surface) return 1;
    const int failed = surface->capacity_failed;
    surface->capacity_failed = 0;
    return failed;
}

int web_surface_get_used_tile_count(const WebPaintSurface *surface)
{
    return surface ? surface->tile_count : 0;
}

int web_surface_get_used_tile_info(const WebPaintSurface *surface, int index,
                                   int *tx, int *ty)
{
    if (!surface || index < 0 || index >= surface->tile_count || !tx || !ty) return 0;
    *tx = surface->used_tile_x[index];
    *ty = surface->used_tile_y[index];
    return 1;
}

uint16_t *web_surface_get_used_tile(WebPaintSurface *surface, int index)
{
    if (!surface || index < 0 || index >= surface->tile_count) return NULL;
    return tile_at_slot(surface, index);
}

void web_surface_set_write_callback(WebPaintSurface *surface,
                                     WebSurfaceWriteCallback callback)
{
    if (surface) surface->write_callback = callback;
}

void web_surface_clear(WebPaintSurface *surface)
{
    if (!surface) return;
    free_tiles(surface);
    memset(surface->tile_used, 0, (size_t)surface->hash_size * sizeof(uint8_t));
    surface->tile_count = 0;
    reset_null_tile(surface);
}

void web_surface_set_symmetry(WebPaintSurface *surface, int active,
                              float center_x, float center_y, float angle,
                              int symmetry_type, int lines)
{
    if (!surface) return;
    mypaint_tiled_surface_set_symmetry_state(
        &surface->parent, active ? 1 : 0, center_x, center_y, angle,
        (MyPaintSymmetryType)symmetry_type, lines);
}
