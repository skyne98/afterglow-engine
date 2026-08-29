/* Maipointo mapping + RNG parity oracle: replays an opcode stream through
 * the REAL mypaint-mapping.c / rng-double.c and emits exact result bits.
 *
 * Stream format (little-endian):
 *   1 MAPPING_NEW      u8 inputs
 *   2 MAPPING_SET_BASE f32
 *   3 MAPPING_SET_N    u8 input, u8 n
 *   4 MAPPING_SET_POINT u8 input, u8 index, f32 x, f32 y
 *   5 MAPPING_CALC     f32 * inputs        -> f32 result
 *   6 RNG_NEW          i64 seed
 *   7 RNG_NEXT         u32 count           -> count * f64
 *
 * Build:
 *   cc -O2 -ffp-contract=off -std=c11 -I<paint> -I<vendor/libmypaint> \
 *      reference/eval_oracle.c ../vendor/libmypaint/mypaint-mapping.c \
 *      ../vendor/libmypaint/rng-double.c -lm -o eval-oracle
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "mypaint-mapping.h"
#include "rng-double.h"
#include "helpers.h"
#include "fastapprox/fastpow.h"

/* render_dab_mask lives in mypaint-tiled-surface.c (not in its header). */
void render_dab_mask(uint16_t * mask,
                     float x, float y,
                     float radius,
                     float hardness,
                     float softness,
                     float aspect_ratio, float angle);
#include "mypaint-config.h" /* MYPAINT_TILE_SIZE */

static uint16_t rec16(const uint8_t *p) { return (uint16_t)(p[0] | (p[1] << 8)); }
static uint32_t rec32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}
static uint64_t rec64(const uint8_t *p) {
    return (uint64_t)rec32(p) | ((uint64_t)rec32(p + 4) << 32);
}

static void put_bytes(FILE *out, const void *data, size_t n) {
    fwrite(data, 1, n, out);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: eval-oracle <commands.bin> <out.bin>\n");
        return 2;
    }
    FILE *in = fopen(argv[1], "rb");
    if (!in) { perror("open"); return 2; }
    FILE *out = fopen(argv[2], "wb");
    if (!out) { perror("open out"); return 2; }

    MyPaintMapping *mapping = NULL;
    RngDouble *rng = NULL;
    int mapping_inputs = 0;
    uint8_t hdr[1];
    uint8_t buf[64];

    while (fread(hdr, 1, 1, in) == 1) {
        switch (hdr[0]) {
        case 20: {
            /* SPECTRAL_RGB: 3x f32 -> 10x f32 (rgb_to_spectral) */
            uint8_t args[12];
            if (fread(args, 1, 12, in) != 12) return 2;
            float c[3];
            memcpy(c, args, 12);
            float spec[10] = {0};
            rgb_to_spectral(c[0], c[1], c[2], spec);
            put_bytes(out, spec, 40);
            break;
        }
        case 21: {
            /* SPECTRAL_TO_RGB: 10x f32 -> 3x f32 */
            uint8_t args[40];
            if (fread(args, 1, 40, in) != 40) return 2;
            float spec[10];
            memcpy(spec, args, 40);
            float rgb[3] = {0};
            spectral_to_rgb(spec, rgb);
            put_bytes(out, rgb, 12);
            break;
        }
        case 22: {
            /* FASTPOW: 2x f32 -> f32 */
            uint8_t args[8];
            if (fread(args, 1, 8, in) != 8) return 8;
            float a, e;
            memcpy(&a, args, 4);
            memcpy(&e, args + 4, 4);
            float r = fastpow(a, e);
            put_bytes(out, &r, 4);
            break;
        }
        case 24: {
            /* SPECTRAL_MIX: 10x f32 a, 10x f32 b, f32 fac_a ->
             * 10x f32 result via fastpow WGM (the paint-mode mix). */
            uint8_t args[84];
            if (fread(args, 1, 84, in) != 84) return 2;
            float sa[10], sb[10], fa;
            memcpy(sa, args, 40);
            memcpy(sb, args + 40, 40);
            memcpy(&fa, args + 80, 4);
            float res[10] = {0};
            for (int i = 0; i < 10; i++) {
                res[i] = fastpow(sa[i], fa) * fastpow(sb[i], 1.0f - fa);
            }
            put_bytes(out, res, 40);
            break;
        }
        case 23: {
            /* MASK: 7x f32 (x,y,radius,hardness,softness,aspect,angle)
             * -> u16 count + LRE mask array */
            uint8_t args[28];
            if (fread(args, 1, 28, in) != 28) return 2;
            float p[7];
            memcpy(p, args, 28);
            static uint16_t mask_buf[MYPAINT_TILE_SIZE * MYPAINT_TILE_SIZE + 2 * MYPAINT_TILE_SIZE];
            memset(mask_buf, 0, sizeof(mask_buf));
            render_dab_mask(mask_buf, p[0], p[1], p[2], p[3], p[4], p[5], p[6]);
            size_t n = 0;
            while (n < sizeof(mask_buf) / 2) {
                if (mask_buf[n] == 0) {
                    if (n + 1 >= sizeof(mask_buf) / 2 || mask_buf[n + 1] == 0) break;
                    n += 2;
                } else {
                    n += 1;
                }
            }
            uint16_t cnt = (uint16_t)n;
            put_bytes(out, &cnt, 2);
            put_bytes(out, mask_buf, n * 2);
            break;
        }
        case 1: {
            uint8_t args[1];
            if (fread(args, 1, 1, in) != 1) return 2;
            if (mapping) mypaint_mapping_free(mapping);
            mapping = mypaint_mapping_new(args[0]);
            mapping_inputs = args[0];
            break;
        }
        case 2: {
            uint8_t args[4];
            if (fread(args, 1, 4, in) != 4) return 2;
            uint32_t bits = rec32(args);
            float v;
            memcpy(&v, &bits, 4);
            mypaint_mapping_set_base_value(mapping, v);
            break;
        }
        case 3: {
            uint8_t args[2];
            if (fread(args, 1, 2, in) != 2) return 2;
            mypaint_mapping_set_n(mapping, args[0], args[1]);
            break;
        }
        case 4: {
            uint8_t args[10];
            if (fread(args, 1, 10, in) != 10) return 2;
            float x, y;
            uint32_t xb = rec32(args + 2), yb = rec32(args + 6);
            memcpy(&x, &xb, 4);
            memcpy(&y, &yb, 4);
            mypaint_mapping_set_point(mapping, args[0], args[1], x, y);
            break;
        }
        case 5: {
            float *data = calloc(mapping_inputs, sizeof(float));
            if (fread(data, sizeof(float), mapping_inputs, in) != (size_t)mapping_inputs) return 2;
            float result = mypaint_mapping_calculate(mapping, data);
            put_bytes(out, &result, 4);
            free(data);
            break;
        }
        case 6: {
            uint8_t args[8];
            if (fread(args, 1, 8, in) != 8) return 2;
            long seed = (long)(int64_t)rec64(args);
            if (rng) rng_double_free(rng);
            rng = rng_double_new(seed);
            break;
        }
        case 7: {
            uint8_t args[4];
            if (fread(args, 1, 4, in) != 4) return 2;
            uint32_t count = rec32(args);
            for (uint32_t i = 0; i < count; i++) {
                double v = rng_double_next(rng);
                put_bytes(out, &v, 8);
            }
            break;
        }
        default:
            fprintf(stderr, "bad opcode %u\n", hdr[0]);
            return 2;
        }
    }
    fclose(in);
    fclose(out);
    return 0;
}
