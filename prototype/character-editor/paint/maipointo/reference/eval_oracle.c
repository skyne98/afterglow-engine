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
