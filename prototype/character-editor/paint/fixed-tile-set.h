#ifndef AFTERGLOW_FIXED_TILE_SET_H
#define AFTERGLOW_FIXED_TILE_SET_H

#include <stdint.h>

typedef struct {
    uint32_t *marks;
    int capacity;
    uint32_t generation;
} FixedTileSet;

void fixed_tile_set_init(FixedTileSet *set, uint32_t *marks, int capacity);
void fixed_tile_set_begin(FixedTileSet *set);
int fixed_tile_set_insert(FixedTileSet *set, int tile_slot);

#endif
