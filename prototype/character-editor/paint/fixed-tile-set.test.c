#include <stdint.h>
#include <stdio.h>

#include "fixed-tile-set.h"

int main(void)
{
    uint32_t marks[4] = { 9, 9, 9, 9 };
    FixedTileSet set;
    fixed_tile_set_init(&set, marks, 4);
    fixed_tile_set_begin(&set);
    if (fixed_tile_set_insert(&set, 2) != 1) return 1;
    if (fixed_tile_set_insert(&set, 2) != 0) return 1;
    if (fixed_tile_set_insert(&set, -1) != -1) return 1;
    if (fixed_tile_set_insert(&set, 4) != -1) return 1;

    fixed_tile_set_begin(&set);
    if (fixed_tile_set_insert(&set, 2) != 1) return 1;

    set.generation = UINT32_MAX;
    marks[0] = 10;
    marks[1] = 20;
    fixed_tile_set_begin(&set);
    if (set.generation != 1 || marks[0] != 0 || marks[1] != 0) return 1;
    if (fixed_tile_set_insert(&set, 0) != 1) return 1;

    uint32_t large_marks[4096] = { 0 };
    FixedTileSet large;
    fixed_tile_set_init(&large, large_marks, 4096);
    fixed_tile_set_begin(&large);
    for (int i = 0; i < 4096; i++) {
        if (fixed_tile_set_insert(&large, i) != 1) return 1;
    }
    for (int i = 4095; i >= 0; i--) {
        if (fixed_tile_set_insert(&large, i) != 0) return 1;
    }

    puts("fixed tile set: generation and capacity correct");
    return 0;
}
