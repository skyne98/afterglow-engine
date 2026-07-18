#pragma once

#include <phonon.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    IPLVector3 v0;
    IPLVector3 v1;
    IPLVector3 v2;
    IPLint32 materialIndex;
} AfterglowObvhsTriangle;

typedef struct {
    uint32_t staticNodeCount;
    uint32_t staticPrimitiveCount;
    uint32_t doorNodeCount;
    uint32_t doorPrimitiveCount;
    uint64_t ownedBytes;
    double buildMilliseconds;
} AfterglowObvhsStats;

/** Builds immutable CWBVH8 acceleration structures and copies all input data. */
int32_t afterglow_obvhs_create(const AfterglowObvhsTriangle* staticTriangles,
                               uint32_t staticTriangleCount,
                               const AfterglowObvhsTriangle* doorTriangles,
                               uint32_t doorTriangleCount,
                               const IPLMaterial* materials,
                               uint32_t materialCount,
                               void** tracer);
void afterglow_obvhs_destroy(void* tracer);

/** Translation-only BLAS instance update; performs no allocation or BVH rebuild. */
void afterglow_obvhs_set_door_y(void* tracer, float doorY);
void afterglow_obvhs_get_stats(const void* tracer, AfterglowObvhsStats* stats);
uint32_t afterglow_obvhs_traversal_lanes(void);

/** Steam Audio IPL_SCENETYPE_CUSTOM callbacks. */
void afterglow_obvhs_closest_hit(const IPLRay* ray, IPLfloat32 minDistance,
                                 IPLfloat32 maxDistance, IPLHit* hit, void* userData);
void afterglow_obvhs_any_hit(const IPLRay* ray, IPLfloat32 minDistance,
                             IPLfloat32 maxDistance, IPLuint8* occluded, void* userData);
void afterglow_obvhs_batched_closest_hit(IPLint32 numRays, const IPLRay* rays,
                                         const IPLfloat32* minDistances,
                                         const IPLfloat32* maxDistances,
                                         IPLHit* hits, void* userData);
void afterglow_obvhs_batched_any_hit(IPLint32 numRays, const IPLRay* rays,
                                     const IPLfloat32* minDistances,
                                     const IPLfloat32* maxDistances,
                                     IPLuint8* occluded, void* userData);

#ifdef __cplusplus
}
#endif
