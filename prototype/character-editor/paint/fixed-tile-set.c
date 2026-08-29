#include "fixed-tile-set.h"

#include <stddef.h>
#include <string.h>

void fixed_tile_set_init(FixedTileSet *set, uint32_t *marks, int capacity)
{
    if (!set) return;
    set->marks = marks;
    set->capacity = capacity > 0 ? capacity : 0;
    set->generation = 0;
    if (marks && capacity > 0) {
        memset(marks, 0, (size_t)capacity * sizeof(uint32_t));
    }
}

void fixed_tile_set_begin(FixedTileSet *set)
{
    if (!set || !set->marks || set->capacity <= 0) return;
    set->generation++;
    if (set->generation == 0) {
        memset(set->marks, 0,
               (size_t)set->capacity * sizeof(uint32_t));
        set->generation = 1;
    }
}

int fixed_tile_set_insert(FixedTileSet *set, int tile_slot)
{
    if (!set || !set->marks || set->generation == 0 || tile_slot < 0 ||
        tile_slot >= set->capacity) {
        return -1;
    }
    if (set->marks[tile_slot] == set->generation) return 0;
    set->marks[tile_slot] = set->generation;
    return 1;
}
