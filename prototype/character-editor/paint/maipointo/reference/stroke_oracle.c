/* Maipointo stroke parity oracle: replays a brush-config + stroke-event
 * stream through the REAL vendored libmypaint C engine on a
 * MyPaintFixedTiledSurface, then dumps the full tile buffer.
 *
 * Stream (little-endian):
 *   0 FROM_DEFAULTS
 *   1 SET_BASE   u16 setting, f32 value
 *   2 SET_N      u16 setting, u16 input, u8 n
 *   3 SET_POINT  u16 setting, u16 input, u8 index, f32 x, f32 y
 *   4 STROKE_TO  f32 x, y, pressure, xtilt, ytilt, f64 dtime,
 *                f32 viewzoom, viewrotation, barrel_rotation, u8 linear
 *
 * Usage: stroke-oracle <w> <h> <commands.bin> <out.bin>
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mypaint-brush.h"
#include "mypaint-fixed-tiled-surface.h"

/* process_tile is non-static in mypaint-tiled-surface.c */
void process_tile(MyPaintTiledSurface *self, int tx, int ty);

static uint16_t rec16(const uint8_t *p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static uint32_t rec32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}
static float recf(const uint8_t *p) {
    float v;
    uint32_t b = rec32(p);
    memcpy(&v, &b, 4);
    return v;
}

int main(int argc, char **argv) {
    if (argc != 5) {
        fprintf(stderr, "usage: stroke-oracle <w> <h> <commands.bin> <out.bin>\n");
        return 2;
    }
    int w = atoi(argv[1]);
    int h = atoi(argv[2]);

    FILE *in = fopen(argv[3], "rb");
    if (!in) { perror("open"); return 2; }
    FILE *out = fopen(argv[4], "wb");
    if (!out) { perror("open out"); return 2; }

    MyPaintBrush *brush = mypaint_brush_new();
    MyPaintFixedTiledSurface *surface = mypaint_fixed_tiled_surface_new(w, h);
    MyPaintSurface *s = (MyPaintSurface *)surface;

    uint8_t rec[40];
    while (fread(rec, 1, 1, in) == 1) {
        switch (rec[0]) {
        case 0:
            mypaint_brush_from_defaults(brush);
            break;
        case 1: {
            uint8_t args[6];
            if (fread(args, 1, 6, in) != 6) return 2;
            mypaint_brush_set_base_value(brush, (MyPaintBrushSetting)rec16(args),
                                         recf(args + 2));
            break;
        }
        case 2: {
            uint8_t args[5];
            if (fread(args, 1, 5, in) != 5) return 2;
            mypaint_brush_set_mapping_n(brush, (MyPaintBrushSetting)rec16(args),
                                        (MyPaintBrushInput)rec16(args + 2), args[4]);
            break;
        }
        case 3: {
            uint8_t args[12];
            if (fread(args, 1, 12, in) != 12) return 2;
            mypaint_brush_set_mapping_point(brush, (MyPaintBrushSetting)rec16(args),
                                            (MyPaintBrushInput)rec16(args + 2),
                                            args[4], recf(args + 5), recf(args + 9));
            break;
        }
        case 4: {
            uint8_t args[38];
            if (fread(args, 1, 38, in) != 38) return 2;
            float x = recf(args);
            float y = recf(args + 4);
            float pressure = recf(args + 8);
            float xtilt = recf(args + 12);
            float ytilt = recf(args + 16);
            double dtime;
            uint64_t db = (uint64_t)rec32(args + 20) |
                          ((uint64_t)rec32(args + 24) << 32);
            memcpy(&dtime, &db, 8);
            float viewzoom = recf(args + 28);
            float viewrotation = recf(args + 32);
            float barrel = recf(args + 36);
            uint8_t lin[1];
            if (fread(lin, 1, 1, in) != 1) return 2;
            mypaint_brush_stroke_to(brush, s, x, y, pressure, xtilt, ytilt,
                                    dtime, viewzoom, viewrotation, barrel,
                                    (gboolean)lin[0]);
            break;
        }
        default:
            fprintf(stderr, "bad opcode %u\n", rec[0]);
            return 2;
        }
    }
    fclose(in);

    int tiles_w = (w + MYPAINT_TILE_SIZE - 1) / MYPAINT_TILE_SIZE;
    int tiles_h = (h + MYPAINT_TILE_SIZE - 1) / MYPAINT_TILE_SIZE;
    for (int ty = 0; ty < tiles_h; ty++) {
        for (int tx = 0; tx < tiles_w; tx++) {
            process_tile((MyPaintTiledSurface *)surface, tx, ty);
        }
    }

    for (int ty = 0; ty < tiles_h; ty++) {
        for (int tx = 0; tx < tiles_w; tx++) {
            MyPaintTileRequest req;
            mypaint_tile_request_init(&req, 0, tx, ty, FALSE);
            mypaint_tiled_surface_tile_request_start((MyPaintTiledSurface *)surface, &req);
            fwrite(req.buffer, 1,
                   MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE * 4 * sizeof(uint16_t), out);
            mypaint_tiled_surface_tile_request_end((MyPaintTiledSurface *)surface, &req);
        }
    }
    fclose(out);
    return 0;
}
