#include <stdio.h>
#include <stdlib.h>

#include "fixed-operationqueue.h"

static OperationDataDrawDab *new_operation(int id)
{
    OperationDataDrawDab *operation = calloc(1, sizeof(*operation));
    if (operation) operation->x = (float)id;
    return operation;
}

int main(void)
{
    OperationQueue *queue = operation_queue_new();
    if (!queue) return 1;
    for (int i = 0; i < 4096; i++) {
        const TileIndex tile = { i & 255, i >> 8 };
        operation_queue_add(queue, tile, new_operation(i));
    }
    TileIndex *dirty = NULL;
    if (operation_queue_failed(queue) ||
        operation_queue_get_dirty_tiles(queue, &dirty) != 4096) return 1;
    for (int i = 0; i < 4096; i++) {
        OperationDataDrawDab *operation = operation_queue_pop(queue, dirty[i]);
        if (!operation || operation->x != (float)i) return 1;
        free(operation);
    }
    const TileIndex extra = { 500, 500 };
    operation_queue_add(queue, extra, new_operation(5000));
    if (!operation_queue_failed(queue)) return 1;
    operation_queue_clear_dirty_tiles(queue);
    if (operation_queue_failed(queue)) return 1;

    const TileIndex same = { 7, 9 };
    for (int i = 0; i < 16385; i++) {
        operation_queue_add(queue, same, new_operation(i));
    }
    if (!operation_queue_failed(queue)) return 1;
    operation_queue_clear_dirty_tiles(queue);
    operation_queue_free(queue);
    puts("fixed operation queue: hash and capacities correct");
    return 0;
}
