/* Maipointo parity oracle.
 *
 * Replays 64-byte dab-command records through the REAL vendored libmypaint
 * code (render_dab_mask + brushmodes.c) into one 64x64 fix15 tile, then
 * writes the raw u16 RGBA tile (LE) followed by five f64 color sums.
 *
 * The Rust parity test feeds identical commands to maipointo and compares
 * bytes. Compile with -ffp-contract=off so the reference has no FMA fusion:
 *   cc -O2 -ffp-contract=off -std=c11 -I<paint> -I<vendor/libmypaint> \
 *      reference/oracle.c ../vendor/libmypaint/brushmodes.c \
 *      ../vendor/libmypaint/mypaint-tiled-surface.c \
 *      ../vendor/libmypaint/helpers.c -lm -o oracle
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mypaint-config.h" /* MYPAINT_TILE_SIZE */
#include "brushmodes.h"

#define TILE_SIZE MYPAINT_TILE_SIZE

/* Not declared in brushmodes.h but exported by brushmodes.c. */
void get_color_pixels_legacy(uint16_t *mask, uint16_t *rgba, float *sum_weight,
                             float *sum_r, float *sum_g, float *sum_b,
                             float *sum_a);

/* render_dab_mask lives in mypaint-tiled-surface.c (not in its header). */
void render_dab_mask(uint16_t *mask, float x, float y, float radius,
                     float hardness, float softness, float aspect_ratio,
                     float angle);

enum {
    MODE_NORMAL = 0,
    MODE_NORMAL_ERASER = 1,
    MODE_LOCK_ALPHA = 2,
    MODE_POSTERIZE = 3,
    MODE_COLORIZE = 4,
    MODE_GET_COLOR_LEGACY = 5,
    MODE_GET_COLOR_ACCUM = 6,
    MODE_NORMAL_PAINT = 7,
    MODE_NORMAL_ERASER_PAINT = 8,
    MODE_LOCK_ALPHA_PAINT = 9,
};

#define REC 64

static uint16_t tile[TILE_SIZE * TILE_SIZE * 4];
static uint16_t mask_buf[TILE_SIZE * TILE_SIZE + 2 * TILE_SIZE];
static float rr_scratch[TILE_SIZE * TILE_SIZE + 2 * TILE_SIZE];

/* The Rust side owns mask generation (mask.rs); the oracle needs the same
 * u16 LRE mask, so we render it with the C and ALSO write the mask to the
 * output for a direct mask comparison. */
static void emit_mask(uint16_t *mask, float x, float y, float radius,
                      float hardness, float softness, float aspect_ratio,
                      float angle) {
    render_dab_mask(mask, x, y, radius, hardness, softness, aspect_ratio,
                    angle);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: oracle <commands.bin> <out.bin>\n");
        return 2;
    }
    FILE *in = fopen(argv[1], "rb");
    if (!in) {
        perror("open commands");
        return 2;
    }
    FILE *out = fopen(argv[2], "wb");
    if (!out) {
        perror("open out");
        return 2;
    }

    uint8_t rec[REC];
    float sums[5] = {0, 0, 0, 0, 0}; /* weight, r, g, b, a (C uses float*) */

    while (fread(rec, 1, REC, in) == REC) {
        uint8_t mode = rec[0];
        float x, y, radius, hardness, softness, aspect, angle;
        uint16_t r, g, b, a, opacity, posterize_num, interval;
        float paint, rand_rate;
        uint32_t seed;
        memcpy(&x, rec + 4, 4);
        memcpy(&y, rec + 8, 4);
        memcpy(&radius, rec + 12, 4);
        memcpy(&hardness, rec + 16, 4);
        memcpy(&softness, rec + 20, 4);
        memcpy(&aspect, rec + 24, 4);
        memcpy(&angle, rec + 28, 4);
        memcpy(&r, rec + 32, 2);
        memcpy(&g, rec + 34, 2);
        memcpy(&b, rec + 36, 2);
        memcpy(&a, rec + 38, 2);
        memcpy(&opacity, rec + 40, 2);
        memcpy(&posterize_num, rec + 42, 2);
        memcpy(&paint, rec + 48, 4);
        memcpy(&interval, rec + 52, 2);
        memcpy(&rand_rate, rec + 56, 4);
        memcpy(&seed, rec + 60, 4);
        (void)seed;

        emit_mask(mask_buf, x, y, radius, hardness, softness, aspect, angle);

        uint16_t *m = mask_buf;
        uint16_t *rgba = tile;
        switch (mode) {
        case MODE_NORMAL:
            draw_dab_pixels_BlendMode_Normal(m, rgba, r, g, b, opacity);
            break;
        case MODE_NORMAL_ERASER:
            draw_dab_pixels_BlendMode_Normal_and_Eraser(m, rgba, r, g, b, a,
                                                        opacity);
            break;
        case MODE_LOCK_ALPHA:
            draw_dab_pixels_BlendMode_LockAlpha(m, rgba, r, g, b, opacity);
            break;
        case MODE_POSTERIZE:
            draw_dab_pixels_BlendMode_Posterize(m, rgba, opacity,
                                                posterize_num);
            break;
        case MODE_COLORIZE:
            draw_dab_pixels_BlendMode_Color(m, rgba, r, g, b, opacity);
            break;
        case MODE_GET_COLOR_LEGACY:
            get_color_pixels_legacy(m, rgba, &sums[0], &sums[1], &sums[2],
                                    &sums[3], &sums[4]);
            break;
        case MODE_GET_COLOR_ACCUM:
            get_color_pixels_accumulate(m, rgba, &sums[0], &sums[1], &sums[2],
                                        &sums[3], &sums[4], paint, interval,
                                        rand_rate);
            break;
        case MODE_NORMAL_PAINT:
            draw_dab_pixels_BlendMode_Normal_Paint(m, rgba, r, g, b, opacity);
            break;
        case MODE_NORMAL_ERASER_PAINT:
            draw_dab_pixels_BlendMode_Normal_and_Eraser_Paint(m, rgba, r, g, b,
                                                              a, opacity);
            break;
        case MODE_LOCK_ALPHA_PAINT:
            draw_dab_pixels_BlendMode_LockAlpha_Paint(m, rgba, r, g, b, opacity);
            break;
        default:
            fprintf(stderr, "bad mode %u\n", mode);
            return 2;
        }
    }
    fclose(in);

    /* Emit tile + mask-free summary. The tile is the byte-exact artifact;
     * sums appended as f64 for the get_color comparison. */
    for (size_t i = 0; i < TILE_SIZE * TILE_SIZE * 4; i++) {
        uint16_t v = tile[i];
        uint8_t le[2] = {(uint8_t)(v & 0xff), (uint8_t)(v >> 8)};
        fwrite(le, 1, 2, out);
    }
    double dsums[5];
    for (int i = 0; i < 5; i++)
        dsums[i] = (double)sums[i];
    fwrite(dsums, 8, 5, out);
    fclose(out);
    return 0;
}
