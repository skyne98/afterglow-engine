#ifndef AFTERGLOW_FIXED_OPERATION_QUEUE_H
#define AFTERGLOW_FIXED_OPERATION_QUEUE_H

#include "operationqueue.h"

OperationDataDrawDab *operation_queue_acquire(OperationQueue *queue);
void operation_queue_release(OperationQueue *queue,
                             OperationDataDrawDab *operation);
int operation_queue_failed(OperationQueue *queue);

#endif
