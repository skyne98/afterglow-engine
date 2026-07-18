#include <phonon.h>
#include <afterglow_obvhs_tracer.h>
#include <emscripten/emscripten.h>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <vector>

namespace {
constexpr std::array<char, 8> kMagic{'A', 'G', 'B', 'I', 'S', 'T', '1', '\0'};
constexpr int kSources = 64;

struct BistroHeader {
    std::array<char, 8> magic;
    uint32_t version;
    uint32_t vertexCount;
    uint32_t triangleCount;
    uint32_t materialCount;
    float minimum[3];
    float maximum[3];
    float listener[3];
    float source[3];
};
static_assert(sizeof(BistroHeader) == 72);

IPLContext gContext = nullptr;
IPLScene gScene = nullptr;
void* gTracer = nullptr;
IPLSimulator gSimulator = nullptr;
std::vector<IPLSource> gSources;
std::vector<IPLSimulationInputs> gInputs;
std::vector<IPLSimulationOutputs> gOutputs;
BistroHeader gHeader{};

IPLCoordinateSpace3 coordinates(float x, float y, float z) {
    IPLCoordinateSpace3 value{};
    value.ahead = {0.0f, 0.0f, -1.0f};
    value.up = {0.0f, 1.0f, 0.0f};
    value.right = {1.0f, 0.0f, 0.0f};
    value.origin = {x, y, z};
    return value;
}

void releaseAll() {
    if (gSimulator) {
        for (auto& source : gSources) {
            if (!source) continue;
            iplSourceRemove(source, gSimulator);
            iplSourceRelease(&source);
        }
        iplSimulatorRelease(&gSimulator);
    }
    gSources.clear();
    gInputs.clear();
    gOutputs.clear();
    if (gScene) iplSceneRelease(&gScene);
    if (gTracer) {
        afterglow_obvhs_destroy(gTracer);
        gTracer = nullptr;
    }
    if (gContext) iplContextRelease(&gContext);
    gHeader = {};
}

bool exactGeometrySize(const BistroHeader& header, uint32_t byteCount) {
    const uint64_t positions = static_cast<uint64_t>(header.vertexCount) * 3 * sizeof(float);
    const uint64_t indices = static_cast<uint64_t>(header.triangleCount) * 3 * sizeof(uint32_t);
    const uint64_t materials = header.triangleCount;
    return sizeof(BistroHeader) + positions + indices + materials == byteCount;
}
}

extern "C" {

EMSCRIPTEN_KEEPALIVE int bistro_init(const uint8_t* bytes, uint32_t byteCount) {
    releaseAll();
    if (!bytes || byteCount < sizeof(BistroHeader)) return 100;
    std::memcpy(&gHeader, bytes, sizeof(gHeader));
    if (gHeader.magic != kMagic || gHeader.version != 1 ||
        gHeader.vertexCount == 0 || gHeader.triangleCount == 0 ||
        gHeader.materialCount != 6 || !exactGeometrySize(gHeader, byteCount)) return 101;

    const auto* vertices = reinterpret_cast<const IPLVector3*>(bytes + sizeof(BistroHeader));
    const auto* indices = reinterpret_cast<const uint32_t*>(vertices + gHeader.vertexCount);
    const auto* materialIndices = reinterpret_cast<const uint8_t*>(indices +
        static_cast<uint64_t>(gHeader.triangleCount) * 3);
    std::array<IPLMaterial, 6> materials{{
        {{0.20f, 0.30f, 0.40f}, 0.20f, {0.05f, 0.03f, 0.02f}},
        {{0.10f, 0.05f, 0.02f}, 0.05f, {0.10f, 0.05f, 0.02f}},
        {{0.40f, 0.60f, 0.70f}, 0.50f, {0.02f, 0.01f, 0.01f}},
        {{0.15f, 0.11f, 0.10f}, 0.30f, {0.03f, 0.02f, 0.01f}},
        {{0.05f, 0.04f, 0.03f}, 0.10f, {0.01f, 0.01f, 0.01f}},
        {{0.10f, 0.05f, 0.03f}, 0.20f, {0.02f, 0.01f, 0.01f}},
    }};
    if (afterglow_obvhs_create_indexed(vertices, gHeader.vertexCount, indices,
                                       materialIndices, gHeader.triangleCount,
                                       materials.data(), materials.size(), &gTracer) != 0) return 2;

    IPLContextSettings contextSettings{};
    contextSettings.version = STEAMAUDIO_VERSION;
    contextSettings.simdLevel = IPL_SIMDLEVEL_NEON;
    if (iplContextCreate(&contextSettings, &gContext) != IPL_STATUS_SUCCESS) return 3;

    IPLSceneSettings sceneSettings{};
    sceneSettings.type = IPL_SCENETYPE_CUSTOM;
    sceneSettings.closestHitCallback = afterglow_obvhs_closest_hit;
    sceneSettings.anyHitCallback = afterglow_obvhs_any_hit;
    sceneSettings.batchedClosestHitCallback = afterglow_obvhs_batched_closest_hit;
    sceneSettings.batchedAnyHitCallback = afterglow_obvhs_batched_any_hit;
    sceneSettings.userData = gTracer;
    if (iplSceneCreate(gContext, &sceneSettings, &gScene) != IPL_STATUS_SUCCESS) return 4;

    IPLSimulationSettings settings{};
    settings.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
    settings.sceneType = IPL_SCENETYPE_CUSTOM;
    settings.reflectionType = IPL_REFLECTIONEFFECTTYPE_PARAMETRIC;
    settings.maxNumRays = 1024;
    settings.numDiffuseSamples = 32;
    settings.maxDuration = 0.5f;
    settings.maxOrder = 0;
    settings.maxNumSources = kSources;
    settings.numThreads = 2;
    settings.rayBatchSize = 64;
    settings.samplingRate = 48000;
    settings.frameSize = 128;
    if (iplSimulatorCreate(gContext, &settings, &gSimulator) != IPL_STATUS_SUCCESS) return 5;
    iplSimulatorSetScene(gSimulator, gScene);
    iplSimulatorCommit(gSimulator);

    IPLSourceSettings sourceSettings{};
    sourceSettings.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
    gSources.resize(kSources, nullptr);
    gInputs.resize(kSources);
    gOutputs.resize(kSources);
    for (int index = 0; index < kSources; ++index) {
        if (iplSourceCreate(gSimulator, &sourceSettings, &gSources[index]) != IPL_STATUS_SUCCESS) return 6;
        iplSourceAdd(gSources[index], gSimulator);
        auto& input = gInputs[index];
        input.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
        input.source = coordinates(gHeader.source[0] + static_cast<float>(index % 8) * 0.08f,
                                   gHeader.source[1],
                                   gHeader.source[2] + static_cast<float>(index / 8) * 0.08f);
        input.reverbScale[0] = input.reverbScale[1] = input.reverbScale[2] = 1.0f;
        input.baked = IPL_FALSE;
        iplSourceSetInputs(gSources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &input);
    }
    iplSimulatorCommit(gSimulator);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int bistro_run_reflections(int rays, float phase) {
    if (!gSimulator || (rays != 512 && rays != 1024) || !std::isfinite(phase)) return 102;
    IPLSimulationSharedInputs shared{};
    shared.listener = coordinates(gHeader.listener[0] + std::sin(phase) * 0.05f,
                                  gHeader.listener[1],
                                  gHeader.listener[2] + std::cos(phase) * 0.05f);
    shared.numRays = rays;
    shared.numBounces = 2;
    shared.duration = 0.5f;
    shared.order = 0;
    shared.irradianceMinDistance = 1.0f;
    iplSimulatorSetSharedInputs(gSimulator, IPL_SIMULATIONFLAGS_REFLECTIONS, &shared);
    iplSimulatorRunReflections(gSimulator);
    for (int index = 0; index < kSources; ++index)
        iplSourceGetOutputs(gSources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &gOutputs[index]);
    return 0;
}

EMSCRIPTEN_KEEPALIVE uint32_t bistro_get_vertices() { return gHeader.vertexCount; }
EMSCRIPTEN_KEEPALIVE uint32_t bistro_get_triangles() { return gHeader.triangleCount; }
EMSCRIPTEN_KEEPALIVE uint32_t bistro_get_tracer_nodes() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return stats.staticNodeCount;
}
EMSCRIPTEN_KEEPALIVE double bistro_get_tracer_build_ms() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return stats.buildMilliseconds;
}
EMSCRIPTEN_KEEPALIVE double bistro_get_tracer_owned_bytes() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return static_cast<double>(stats.ownedBytes);
}
EMSCRIPTEN_KEEPALIVE float bistro_get_reverb_low() {
    return gOutputs.empty() ? 0.0f : gOutputs[0].reflections.reverbTimes[0];
}
EMSCRIPTEN_KEEPALIVE int bistro_get_ir_valid() {
    return (!gOutputs.empty() && gOutputs[0].reflections.ir) ? 1 : 0;
}
EMSCRIPTEN_KEEPALIVE uint32_t bistro_get_simulation_threads() { return 2; }
EMSCRIPTEN_KEEPALIVE uint32_t bistro_get_tracer_lanes() {
    return afterglow_obvhs_traversal_lanes();
}
EMSCRIPTEN_KEEPALIVE void bistro_shutdown() { releaseAll(); }

}
