#include "web-surface.h"

#include <math.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "operationqueue.h"

/* External symbol from the vendored libmypaint tiled surface. Not edited.
 * It pops the tile's ops from the operation queue, fetches the tile through
 * tile_request_start, stamps dabs, and frees the ops. Tiles are independent,
 * so each worker can own a disjoint set of tiles. */
extern void process_tile(MyPaintTiledSurface *self, int tx, int ty);

#define WEB_SURFACE_MIN_HASH_SIZE 8192
#define WEB_SURFACE_NULL_TILES 16

struct WebPaintSurface {
    MyPaintTiledSurface parent;
    pthread_mutex_t lock;
    uint16_t **tiles;
    uint16_t **null_tiles;
    int null_tile_rot;
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
            __atomic_store_n(&surface->capacity_failed, 1, __ATOMIC_SEQ_CST);
            return -1;
        }
        uint16_t *tile = (uint16_t *)calloc(1, surface->tile_bytes);
        if (!tile) {
            __atomic_store_n(&surface->capacity_failed, 1, __ATOMIC_SEQ_CST);
            return -1;
        }
        const int tile_slot = __atomic_fetch_add(&surface->tile_count, 1, __ATOMIC_SEQ_CST);
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

static int no_create_flag = 0;
static volatile int tile_spinlock = 0;

int web_surface_get_no_create(WebPaintSurface *surface) { return no_create_flag; }
void web_surface_set_no_create(WebPaintSurface *surface, int v) { no_create_flag = v; }

static uint16_t *null_tile_for(WebPaintSurface *surface, int index)
{
    if (index < 0) index = 0;
    return surface->null_tiles[index % WEB_SURFACE_NULL_TILES];
}

static void reset_null_tile(uint16_t *tile, size_t tile_bytes)
{
    if (tile) memset(tile, 0, tile_bytes);
}

static void tile_request_start(MyPaintTiledSurface *tiled_surface,
                               MyPaintTileRequest *request)
{
    WebPaintSurface *surface = (WebPaintSurface *)tiled_surface;
    const int tx = request->tx;
    const int ty = request->ty;
    /* Spinlock protects the hash table from concurrent create_tile_slot races.
     * Uses __atomic_test_and_set (no Atomics.wait, can't deadlock). */
    while (__atomic_test_and_set(&tile_spinlock, __ATOMIC_ACQUIRE)) {}
    if (request->mipmap_level != 0) {
        uint16_t *null_tile = null_tile_for(surface, __atomic_fetch_add(&surface->null_tile_rot, 1, __ATOMIC_SEQ_CST));
        request->buffer = null_tile;
        request->context = (gpointer)null_tile;
        __atomic_clear(&tile_spinlock, __ATOMIC_RELEASE);
        return;
    }

    int tile_slot = find_tile_slot(surface, tx, ty);
    /* From a pthread worker: never create tiles (calloc deadlocks on
     * wasmMemory.grow proxy). Pre-creation on the main thread handles this. */
    if (tile_slot < 0 && !request->readonly && !web_surface_get_no_create(surface)) {
        tile_slot = create_tile_slot(surface, tx, ty);
    }
    if (tile_slot < 0) {
        uint16_t *null_tile = null_tile_for(surface, __atomic_fetch_add(&surface->null_tile_rot, 1, __ATOMIC_SEQ_CST));
        request->buffer = null_tile;
        request->context = (gpointer)null_tile;
        __atomic_clear(&tile_spinlock, __ATOMIC_RELEASE);
        return;
    }
    request->buffer = tile_at_slot(surface, tile_slot);
    request->context = NULL;
    if (!request->readonly && surface->write_callback) {
        surface->write_callback(surface, tx, ty, request->buffer);
    }
    __atomic_clear(&tile_spinlock, __ATOMIC_RELEASE);
}

static void tile_request_end(MyPaintTiledSurface *tiled_surface,
                             MyPaintTileRequest *request)
{
    WebPaintSurface *surface = (WebPaintSurface *)tiled_surface;
    if (request->context) {
        reset_null_tile((uint16_t *)request->context, surface->tile_bytes);
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
    if (surface->null_tiles) {
        for (int i = 0; i < WEB_SURFACE_NULL_TILES; i++) free(surface->null_tiles[i]);
        free(surface->null_tiles);
    }
    pthread_mutex_destroy(&surface->lock);
    free(surface);
}

WebPaintSurface *web_surface_new(int width, int height)
{
    if (width <= 0 || height <= 0) return NULL;

    WebPaintSurface *surface = (WebPaintSurface *)calloc(1, sizeof(WebPaintSurface));
    if (!surface) return NULL;

    mypaint_tiled_surface_init(&surface->parent, tile_request_start, tile_request_end);
    surface->parent.parent.destroy = web_surface_free;
    surface->parent.threadsafe_tile_requests = TRUE;
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
    surface->null_tiles = (uint16_t **)calloc((size_t)WEB_SURFACE_NULL_TILES, sizeof(uint16_t *));
    for (int i = 0; i < WEB_SURFACE_NULL_TILES; i++) {
        surface->null_tiles[i] = (uint16_t *)calloc(1, surface->tile_bytes);
    }
    pthread_mutex_init(&surface->lock, NULL);
    if (!surface->tiles || !surface->tile_used || !surface->used_tile_x ||
        !surface->used_tile_y || !surface->tile_x || !surface->tile_y ||
        !surface->tile_slot || !surface->null_tiles) {
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
    for (int i = 0; i < WEB_SURFACE_NULL_TILES; i++) {
        reset_null_tile(surface->null_tiles[i], surface->tile_bytes);
    }
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

/* ------------------------------------------------------------------------
 * Parallel batch driver.
 *
 * Design rules (proven on the 680M):
 *  - No Atomics.wait, no pthread_join, no pthread_mutex_lock. Workers spin
 *    only on claim/done counters; the main runtime thread stays live between
 *    the 2 ms polls that call batch_is_done.
 *  - Workers never allocate tile memory: every dirty tile is pre-created on
 *    the main thread (batch_precreate), so tile_request_start under
 *    no_create=1 always finds an existing tile.
 *  - Workers free() the per-dab operation copies; Emscripten proxies sbrk to
 *    the main thread, which is live during the polls.
 *  - The operation queue is quiescent during a batch (dabs stopped), and each
 *    tile's FIFO is popped by one worker only (atomic claim), so no lock is
 *    needed on the queue.
 *  - One batch at a time; the TS worker serializes strokes around in_flight.
 * ---------------------------------------------------------------------- */

typedef struct {
    WebPaintSurface *surface;
    TileIndex *tiles;   /* read-only pointer into the queue's dirty list */
    int count;
    int target;
    volatile int claim;
    volatile int done;
    volatile int in_flight;
    volatile int disabled;   /* watchdog aborted threading; serial fallback */
} ParallelBatch;

static ParallelBatch g_batch;

static void *paint_worker_main(void *arg)
{
    (void)arg;
    for (;;) {
        const int i = __atomic_fetch_add(&g_batch.claim, 1, __ATOMIC_RELAXED);
        if (i >= g_batch.count) break;
        process_tile(&g_batch.surface->parent,
                     g_batch.tiles[i].x, g_batch.tiles[i].y);
    }
    __atomic_add_fetch(&g_batch.done, 1, __ATOMIC_RELEASE);
    return NULL;
}

int web_surface_batch_begin(WebPaintSurface *surface)
{
    if (!surface) return 0;
    return 1;
}

int web_surface_batch_precreate(WebPaintSurface *surface)
{
    if (!surface) return 0;
    TileIndex *tiles = NULL;
    const int count = operation_queue_get_dirty_tiles(
        surface->parent.operation_queue, &tiles);
    web_surface_set_no_create(surface, 0);
    MyPaintTileRequest request;
    for (int i = 0; i < count; i++) {
        /* The request path creates missing tiles and fires the history
         * write-callback exactly as serial processing would. */
        mypaint_tile_request_init(&request, 0, tiles[i].x, tiles[i].y, FALSE);
        mypaint_tiled_surface_tile_request_start(&surface->parent, &request);
        mypaint_tiled_surface_tile_request_end(&surface->parent, &request);
    }
    return count;
}

int web_surface_batch_launch(WebPaintSurface *surface)
{
    if (!surface || g_batch.in_flight) return -1;
    if (g_batch.disabled) return -1;   /* watchdog stalled once; stay serial */
    g_batch.surface = surface;
    g_batch.count = operation_queue_get_dirty_tiles(
        surface->parent.operation_queue, &g_batch.tiles);
    if (g_batch.count < 2) return -1;   /* nothing worth parallelizing */
    g_batch.target = g_batch.count < WEB_THREAD_POOL_MAX
        ? g_batch.count : WEB_THREAD_POOL_MAX;
    g_batch.claim = 0;
    g_batch.done = 0;
    web_surface_set_no_create(surface, 1);
    __atomic_store_n(&g_batch.in_flight, 1, __ATOMIC_RELEASE);
    /* Spawn workers; count the ones that actually started. Never fall back to
     * serial while a spawned worker still runs (double-process race), so on
     * partial failure we simply wait for the live workers and let finish()
     * process any unclaimed tiles serially. If none started, finish() does
     * all the work (spawned == 0 => is_done true immediately). */
    int spawned = 0;
    for (int i = 0; i < g_batch.target; i++) {
        pthread_t thread;
        if (pthread_create(&thread, NULL, paint_worker_main, NULL) != 0) {
            break;
        }
        pthread_detach(thread);
        spawned++;
    }
    g_batch.target = spawned;
    return 0;
}

int web_surface_batch_is_done(void)
{
    if (!__atomic_load_n(&g_batch.in_flight, __ATOMIC_ACQUIRE)) return 1;
    return __atomic_load_n(&g_batch.done, __ATOMIC_ACQUIRE) >= g_batch.target;
}

int web_surface_batch_in_flight(void)
{
    return __atomic_load_n(&g_batch.in_flight, __ATOMIC_ACQUIRE);
}

int web_surface_batch_abort(WebPaintSurface *surface)
{
    WebPaintSurface *s = g_batch.surface ? g_batch.surface : surface;
    const int dropped = g_batch.count;
    __atomic_store_n(&g_batch.in_flight, 0, __ATOMIC_RELEASE);
    __atomic_store_n(&g_batch.done, g_batch.target, __ATOMIC_RELEASE);
    __atomic_store_n(&g_batch.disabled, 1, __ATOMIC_RELEASE);
    if (s) {
        /* Drop the pending dabs and release no_create so a serial fallback
         * batch (or foreground import) can allocate tiles again. Workers
         * still inside process_tile finish their current tile, then pop NULL
         * because the queue is empty, and exit harmlessly. */
        operation_queue_clear_dirty_tiles(s->parent.operation_queue);
        web_surface_set_no_create(s, 0);
    }
    g_batch.surface = NULL;
    g_batch.tiles = NULL;
    return dropped;
}

int web_surface_batch_finish(WebPaintSurface *surface, MyPaintRectangles *roi)
{
    if (!__atomic_load_n(&g_batch.in_flight, __ATOMIC_ACQUIRE)) return 0;
    WebPaintSurface *s = g_batch.surface ? g_batch.surface : surface;
    /* Claim and process any tiles no worker picked up (e.g. zero workers
     * started). Normally claim == count here and this is a no-op. */
    if (s) {
        for (;;) {
            const int i = __atomic_fetch_add(&g_batch.claim, 1, __ATOMIC_RELAXED);
            if (i >= g_batch.count) break;
            process_tile(&s->parent, g_batch.tiles[i].x, g_batch.tiles[i].y);
        }
    }
    if (!s) return 0;
    /* roi merge: mirrors the vendored mypaint_tiled_surface_end_atomic. */
    if (roi && roi->num_rectangles > 0) {
        const int roi_rects = roi->num_rectangles;
        const int num_dirty = s->parent.num_bboxes_dirtied;
        const int clear_count = roi_rects < num_dirty ? roi_rects : num_dirty;
        for (int i = 0; i < clear_count; i++) {
            roi->rectangles[i].x = 0;
            roi->rectangles[i].y = 0;
            roi->rectangles[i].width = 0;
            roi->rectangles[i].height = 0;
        }
        if (num_dirty > 0) {
            const float bboxes_per_output =
                (float)num_dirty > (float)roi_rects
                ? (float)num_dirty / (float)roi_rects : 1.0f;
            for (int i = 0; i < num_dirty; i++) {
                int out_index = i;
                if (num_dirty > roi_rects) {
                    float index = (float)i / bboxes_per_output;
                    int rounded = (int)(index + 0.5f);
                    if (rounded > roi_rects - 1) rounded = roi_rects - 1;
                    out_index = rounded;
                }
                mypaint_rectangle_expand_to_include_rect(
                    &roi->rectangles[out_index], &s->parent.bboxes[i]);
            }
            roi->num_rectangles = roi_rects < num_dirty ? roi_rects : num_dirty;
        } else {
            roi->num_rectangles = 0;
        }
    }
    operation_queue_clear_dirty_tiles(s->parent.operation_queue);
    web_surface_set_no_create(s, 0);
    __atomic_store_n(&g_batch.in_flight, 0, __ATOMIC_RELEASE);
    g_batch.surface = NULL;
    g_batch.tiles = NULL;
    return roi ? roi->num_rectangles : 0;
}
