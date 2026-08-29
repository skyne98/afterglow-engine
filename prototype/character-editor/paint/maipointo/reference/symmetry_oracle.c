/* Symmetry-transform parity oracle for the maipointo Rust port.
 * stdin: i32 type, f32 cx, f32 cy, f32 angle, i32 lines
 *   (type < 0 means "default state only", no set_pending)
 * stdout: i32 num, then num x 9 f32 (row-major matrix rows).
 * Links the real vendored mypaint-symmetry.c + mypaint-matrix.c. */
#include <stdint.h>
#include <stdio.h>

#include "mypaint-symmetry.h"
#include "mypaint-matrix.h"

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: symmetry-oracle <cases.bin> <out.bin>\n");
        return 2;
    }
    FILE *in = fopen(argv[1], "rb");
    if (!in) { perror("open"); return 2; }
    FILE *out = fopen(argv[2], "wb");
    if (!out) { perror("open out"); return 2; }
    MyPaintSymmetryData data = mypaint_default_symmetry_data();
    int32_t kind;
    float cx, cy, angle;
    int32_t lines;
    while (fread(&kind, 4, 1, in) == 1) {
        if (fread(&cx, 4, 1, in) != 1) return 2;
        if (fread(&cy, 4, 1, in) != 1) return 2;
        if (fread(&angle, 4, 1, in) != 1) return 2;
        if (fread(&lines, 4, 1, in) != 1) return 2;
        if (kind >= 0) {
            mypaint_symmetry_set_pending(&data, 1, cx, cy, angle,
                                         (MyPaintSymmetryType)kind, lines);
        }
        mypaint_update_symmetry_state(&data);
        int32_t n = (int32_t)data.num_symmetry_matrices;
        fwrite(&n, 4, 1, out);
        for (int i = 0; i < n; i++) {
            fwrite(data.symmetry_matrices[i].rows, 4, 9, out);
        }
    }
    fclose(in);
    fclose(out);
    return 0;
}
