/* Fixed-capacity operation queue for the brush engine.
 *
 * The vendored libmypaint draw_dab_internal() mallocs each operation and
 * process_tile() frees it with free(); the queue is only a routing layer.
 * Ops therefore live outside the queue: per-tile FIFOs hold small nodes
 * that point at the caller-owned OperationDataDrawDab objects.
 *
 * The queue has a fixed tile hash and a fixed operation capacity.
 * Adds happen only on the main thread (draw_dab while the
 * engine batches). Popping happens from the parallel batch workers, one
 * worker per tile, so each tile's FIFO is touched by one thread only.
 *
 * op_count is only a soft-cap counter for adds. Popping does NOT decrement
 * it (a plain int would race across workers). operation_queue_clear_dirty_tiles
 * resets it on the main thread after every batch, so the per-batch add budget
 * stays correct.
 */
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include "fixed-operationqueue.h"

#define FIXED_TILE_CAPACITY 4096
#define FIXED_TILE_HASH_CAPACITY 8192
#define FIXED_OP_CAPACITY 16384

typedef struct OpNode {
    OperationDataDrawDab *op;
    struct OpNode *next;
} OpNode;

typedef struct {
    TileIndex index;
    OpNode *head;
    OpNode *tail;
} FixedTileQueue;

struct OperationQueue {
    FixedTileQueue tiles[FIXED_TILE_CAPACITY];
    uint16_t tile_hash_slots[FIXED_TILE_HASH_CAPACITY];
    int tile_count;
    TileIndex dirty_tiles[FIXED_TILE_CAPACITY];
    int dirty_count;
    int op_count;
    int capacity_failed;
};

static int tile_equal(TileIndex a, TileIndex b)
{
    return a.x == b.x && a.y == b.y;
}

static uint32_t tile_hash(TileIndex index)
{
    uint32_t value = (uint32_t)index.x * 0x9E3779B1u;
    value ^= (uint32_t)index.y * 0x85EBCA77u;
    value ^= value >> 16;
    return value * 0xC2B2AE3Du;
}

static int find_tile(OperationQueue *queue, TileIndex index)
{
    const uint32_t start = tile_hash(index) &
        (FIXED_TILE_HASH_CAPACITY - 1u);
    for (uint32_t probe = 0; probe < FIXED_TILE_HASH_CAPACITY; probe++) {
        const uint32_t hash_slot = (start + probe) &
            (FIXED_TILE_HASH_CAPACITY - 1u);
        const uint16_t stored = queue->tile_hash_slots[hash_slot];
        if (stored == 0) return -1;
        const int tile = (int)stored - 1;
        if (tile_equal(queue->tiles[tile].index, index)) return tile;
    }
    return -1;
}

static int insert_tile(OperationQueue *queue, TileIndex index, int tile)
{
    const uint32_t start = tile_hash(index) &
        (FIXED_TILE_HASH_CAPACITY - 1u);
    for (uint32_t probe = 0; probe < FIXED_TILE_HASH_CAPACITY; probe++) {
        const uint32_t hash_slot = (start + probe) &
            (FIXED_TILE_HASH_CAPACITY - 1u);
        if (queue->tile_hash_slots[hash_slot] == 0) {
            queue->tile_hash_slots[hash_slot] = (uint16_t)(tile + 1);
            return 1;
        }
    }
    return 0;
}

OperationQueue *operation_queue_new(void)
{
    OperationQueue *queue = (OperationQueue *)calloc(1, sizeof(OperationQueue));
    return queue;
}

void operation_queue_free(OperationQueue *queue)
{
    if (!queue) return;
    for (int i = 0; i < queue->tile_count; i++) {
        OpNode *node = queue->tiles[i].head;
        while (node) {
            OpNode *next = node->next;
            free(node->op);
            free(node);
            node = next;
        }
    }
    free(queue);
}

/* Compatibility shims: the pristine vendored code does not call these. */
OperationDataDrawDab *operation_queue_acquire(OperationQueue *queue)
{
    if (queue) queue->capacity_failed = 1;
    return NULL;
}

void operation_queue_release(OperationQueue *queue, OperationDataDrawDab *operation)
{
    (void)queue;
    (void)operation;
}

int operation_queue_failed(OperationQueue *queue)
{
    return queue ? queue->capacity_failed : 1;
}

int operation_queue_get_dirty_tiles(OperationQueue *queue, TileIndex **tiles_out)
{
    if (!queue || !tiles_out) return 0;
    *tiles_out = queue->dirty_tiles;
    return queue->dirty_count;
}

void operation_queue_clear_dirty_tiles(OperationQueue *queue)
{
    if (!queue) return;
    for (int i = 0; i < queue->tile_count; i++) {
        OpNode *node = queue->tiles[i].head;
        while (node) {
            OpNode *next = node->next;
            free(node->op);
            free(node);
            node = next;
        }
        queue->tiles[i].head = NULL;
        queue->tiles[i].tail = NULL;
    }
    queue->tile_count = 0;
    memset(queue->tile_hash_slots, 0, sizeof(queue->tile_hash_slots));
    queue->dirty_count = 0;
    queue->op_count = 0;
    queue->capacity_failed = 0;
}

void operation_queue_add(OperationQueue *queue, TileIndex index,
                         OperationDataDrawDab *operation)
{
    if (!queue || !operation) return;
    if (queue->op_count >= FIXED_OP_CAPACITY) {
        queue->capacity_failed = 1;
        free(operation);
        return;
    }
    int tile = find_tile(queue, index);
    if (tile < 0) {
        if (queue->tile_count >= FIXED_TILE_CAPACITY) {
            queue->capacity_failed = 1;
            free(operation);
            return;
        }
        tile = queue->tile_count;
        if (!insert_tile(queue, index, tile)) {
            queue->capacity_failed = 1;
            free(operation);
            return;
        }
        queue->tile_count++;
        queue->tiles[tile].index = index;
        queue->tiles[tile].head = NULL;
        queue->tiles[tile].tail = NULL;
        queue->dirty_tiles[queue->dirty_count++] = index;
    }
    OpNode *node = (OpNode *)malloc(sizeof(OpNode));
    if (!node) {
        queue->capacity_failed = 1;
        free(operation);
        return;
    }
    node->op = operation;
    node->next = NULL;
    if (queue->tiles[tile].tail) {
        queue->tiles[tile].tail->next = node;
    } else {
        queue->tiles[tile].head = node;
    }
    queue->tiles[tile].tail = node;
    queue->op_count++;
}

OperationDataDrawDab *operation_queue_pop(OperationQueue *queue, TileIndex index)
{
    if (!queue) return NULL;
    const int tile = find_tile(queue, index);
    if (tile < 0 || !queue->tiles[tile].head) return NULL;
    OpNode *node = queue->tiles[tile].head;
    queue->tiles[tile].head = node->next;
    if (!queue->tiles[tile].head) queue->tiles[tile].tail = NULL;
    OperationDataDrawDab *op = node->op;
    free(node);
    /* op_count is add-only; see the header comment (no worker race). */
    return op;
}

OperationDataDrawDab *operation_queue_peek_first(OperationQueue *queue, TileIndex index)
{
    if (!queue) return NULL;
    const int tile = find_tile(queue, index);
    if (tile < 0 || !queue->tiles[tile].head) return NULL;
    return queue->tiles[tile].head->op;
}

OperationDataDrawDab *operation_queue_peek_last(OperationQueue *queue, TileIndex index)
{
    if (!queue) return NULL;
    const int tile = find_tile(queue, index);
    if (tile < 0 || !queue->tiles[tile].tail) return NULL;
    return queue->tiles[tile].tail->op;
}
