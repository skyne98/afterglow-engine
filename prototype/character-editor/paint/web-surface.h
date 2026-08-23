#ifndef WEB_SURFACE_H
#define WEB_SURFACE_H

#include <stdint.h>
#include "mypaint-tiled-surface.h"
#include "mypaint-rectangle.h"

#define WEB_SURFACE_MAX_DIRTY_RECTS 32

#define WEB_THREAD_POOL_MAX 4

typedef struct WebPaintSurface WebPaintSurface;
typedef void (*WebSurfaceWriteCallback)(WebPaintSurface *surface, int tx, int ty,
                                        uint16_t *tile);

WebPaintSurface *web_surface_new(int width, int height);
void web_surface_destroy(WebPaintSurface *surface);
MyPaintSurface *web_surface_interface(WebPaintSurface *surface);

int web_surface_get_width(const WebPaintSurface *surface);
int web_surface_get_height(const WebPaintSurface *surface);
int web_surface_get_tiles_width(const WebPaintSurface *surface);
int web_surface_get_tiles_height(const WebPaintSurface *surface);
uint16_t *web_surface_get_tile(WebPaintSurface *surface, int tx, int ty);
uint16_t *web_surface_get_or_create_tile(WebPaintSurface *surface, int tx, int ty);
int web_surface_take_capacity_error(WebPaintSurface *surface);
int web_surface_get_no_create(WebPaintSurface *surface);
void web_surface_set_no_create(WebPaintSurface *surface, int v);
int web_surface_get_used_tile_count(const WebPaintSurface *surface);
int web_surface_get_used_tile_info(const WebPaintSurface *surface, int index,
                                    int *tx, int *ty);
uint16_t *web_surface_get_used_tile(WebPaintSurface *surface, int index);
void web_surface_set_write_callback(WebPaintSurface *surface,
                                     WebSurfaceWriteCallback callback);
void web_surface_clear(WebPaintSurface *surface);
void web_surface_set_symmetry(WebPaintSurface *surface, int active,
                               float center_x, float center_y, float angle,
                               int symmetry_type, int lines);

/* Parallel batch driver. Only the threaded build (web-surface-threads.c)
 * defines these; the serial build never references them.
 *
 * Contract (one active surface at a time):
 *   begin()     main thread, before dabs accumulate
 *   precreate() main thread: create every dirty tile through the request
 *               path so history before-states are captured exactly once
 *   launch()    main thread: spawn the pool over the dirty tiles; returns
 *               0 = async in flight, non-zero = caller must process serially
 *   is_done()   any thread: 1 when all workers finished the current batch
 *   finish()    main thread, only after is_done(): roi merge + queue clear
 *   in_flight() 1 while a parallel batch is running
 */
int web_surface_batch_begin(WebPaintSurface *surface);
int web_surface_batch_precreate(WebPaintSurface *surface);
int web_surface_batch_launch(WebPaintSurface *surface);
int web_surface_batch_is_done(void);
int web_surface_batch_finish(WebPaintSurface *surface, MyPaintRectangles *roi);
int web_surface_batch_in_flight(void);

#endif
