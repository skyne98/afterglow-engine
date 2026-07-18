#include <phonon.h>
#include <afterglow_obvhs_tracer.h>
#if defined(__EMSCRIPTEN__)
#include <emscripten/emscripten.h>
#else
#define EMSCRIPTEN_KEEPALIVE
#endif
#include <cmath>
#include <vector>

namespace {
constexpr int kSampleRate = 48000;
constexpr int kFrameSize = 128;

IPLContext gContext = nullptr;
IPLScene gScene = nullptr;
void* gTracer = nullptr;
IPLSimulator gSimulator = nullptr;
std::vector<IPLSource> gSources;
std::vector<IPLSimulationInputs> gSourceInputs;
std::vector<IPLSimulationOutputs> gOutputs;
std::vector<IPLReflectionEffect> gEffects;
IPLHRTF gHrtf = nullptr;
std::vector<IPLBinauralEffect> gBinauralEffects;
IPLSimulationSharedInputs gSharedInputs{};
IPLAudioBuffer gAudioIn{};
IPLAudioBuffer gAudioOut{};
IPLAudioBuffer gBinauralOut{};
IPLReflectionEffectType gReflectionType = IPL_REFLECTIONEFFECTTYPE_PARAMETRIC;
int gMaxRays = 0;
int gMaxBounces = 0;
int gMaxDurationMs = 0;
int gMaxOrder = 0;
int gSimulationThreads = 1;
float gOutputEnergy = 0.0f;

IPLCoordinateSpace3 coordinates(float x, float y, float z) {
    IPLCoordinateSpace3 value{};
    value.ahead = {0.0f, 0.0f, -1.0f};
    value.up = {0.0f, 1.0f, 0.0f};
    value.right = {1.0f, 0.0f, 0.0f};
    value.origin = {x, y, z};
    return value;
}

void releaseAll() {
    if (gBinauralOut.data) iplAudioBufferFree(gContext, &gBinauralOut);
    if (gAudioOut.data) iplAudioBufferFree(gContext, &gAudioOut);
    if (gAudioIn.data) iplAudioBufferFree(gContext, &gAudioIn);
    for (auto& effect : gBinauralEffects) if (effect) iplBinauralEffectRelease(&effect);
    gBinauralEffects.clear();
    if (gHrtf) iplHRTFRelease(&gHrtf);
    for (auto& effect : gEffects) if (effect) iplReflectionEffectRelease(&effect);
    gEffects.clear();
    for (auto& source : gSources) {
        if (source) {
            iplSourceRemove(source, gSimulator);
            iplSourceRelease(&source);
        }
    }
    gSources.clear();
    gSourceInputs.clear();
    gOutputs.clear();
    if (gSimulator) iplSimulatorRelease(&gSimulator);
    if (gScene) iplSceneRelease(&gScene);
    if (gTracer) {
        afterglow_obvhs_destroy(gTracer);
        gTracer = nullptr;
    }
    if (gContext) iplContextRelease(&gContext);
    gAudioIn = {};
    gAudioOut = {};
    gBinauralOut = {};
}

std::vector<AfterglowObvhsTriangle> flattenTriangles(
    const std::vector<IPLVector3>& vertices,
    const std::vector<IPLTriangle>& triangles,
    IPLint32 materialIndex) {
    std::vector<AfterglowObvhsTriangle> output;
    output.reserve(triangles.size());
    for (const auto& triangle : triangles) {
        output.push_back({vertices[triangle.indices[0]], vertices[triangle.indices[1]],
                          vertices[triangle.indices[2]], materialIndex});
    }
    return output;
}

void addQuad(std::vector<IPLVector3>& vertices, std::vector<IPLTriangle>& triangles,
             IPLVector3 a, IPLVector3 b, IPLVector3 c, IPLVector3 d) {
    const IPLint32 base = static_cast<IPLint32>(vertices.size());
    vertices.push_back(a); vertices.push_back(b); vertices.push_back(c); vertices.push_back(d);
    triangles.push_back({base, base + 1, base + 2});
    triangles.push_back({base, base + 2, base + 3});
}
}

extern "C" {

EMSCRIPTEN_KEEPALIVE int dyn_set_simulation_threads(int threads) {
    if (threads < 1 || threads > 16) return 105;
    gSimulationThreads = threads;
    return 0;
}

EMSCRIPTEN_KEEPALIVE int dyn_init(int triangleCount, int sourceCount, int maxRays,
                                  int maxBounces, int reflectionType,
                                  int maxDurationMs, int maxOrder) {
    releaseAll();
    if (triangleCount < 12 || triangleCount > 1'000'000 ||
        sourceCount < 1 || sourceCount > 128 ||
        maxRays < 1 || maxRays > 65'536 ||
        maxBounces < 1 || maxBounces > 64 ||
        (reflectionType != IPL_REFLECTIONEFFECTTYPE_CONVOLUTION &&
         reflectionType != IPL_REFLECTIONEFFECTTYPE_PARAMETRIC) ||
        maxDurationMs < 1 || maxDurationMs > 4'000 ||
        maxOrder < 0 || maxOrder > 3) return 100;
    gReflectionType = static_cast<IPLReflectionEffectType>(reflectionType);
    gMaxRays = maxRays;
    gMaxBounces = maxBounces;
    gMaxDurationMs = maxDurationMs;
    gMaxOrder = maxOrder;

    IPLContextSettings contextSettings{};
    contextSettings.version = STEAMAUDIO_VERSION;
#if defined(__EMSCRIPTEN__)
    contextSettings.simdLevel = IPL_SIMDLEVEL_NEON;
#else
    contextSettings.simdLevel = IPL_SIMDLEVEL_AVX2;
#endif
    if (iplContextCreate(&contextSettings, &gContext) != IPL_STATUS_SUCCESS) return 1;

    std::vector<IPLVector3> vertices;
    std::vector<IPLTriangle> triangles;
    vertices.reserve(static_cast<size_t>(triangleCount) * 3);
    triangles.reserve(static_cast<size_t>(triangleCount));
    // A 10 m × 4 m × 10 m room. This is runtime geometry, not baked acoustic data.
    addQuad(vertices, triangles, {-5, -2, -5}, { 5, -2, -5}, { 5, -2,  5}, {-5, -2,  5});
    addQuad(vertices, triangles, {-5,  2,  5}, { 5,  2,  5}, { 5,  2, -5}, {-5,  2, -5});
    addQuad(vertices, triangles, {-5, -2,  5}, { 5, -2,  5}, { 5,  2,  5}, {-5,  2,  5});
    addQuad(vertices, triangles, { 5, -2, -5}, {-5, -2, -5}, {-5,  2, -5}, { 5,  2, -5});
    addQuad(vertices, triangles, {-5, -2, -5}, {-5, -2,  5}, {-5,  2,  5}, {-5,  2, -5});
    addQuad(vertices, triangles, { 5, -2,  5}, { 5, -2, -5}, { 5,  2, -5}, { 5,  2,  5});
    while (static_cast<int>(triangles.size()) < triangleCount) {
        const int index = static_cast<int>(triangles.size());
        const float x = 20.0f + static_cast<float>(index % 317) * 0.07f;
        const float y = 20.0f + static_cast<float>((index / 317) % 317) * 0.07f;
        const float z = static_cast<float>(index % 97) * 0.03f;
        const IPLint32 base = static_cast<IPLint32>(vertices.size());
        vertices.push_back({x, y, z});
        vertices.push_back({x + 0.02f, y, z});
        vertices.push_back({x, y + 0.02f, z});
        triangles.push_back({base, base + 1, base + 2});
    }
    std::vector<IPLVector3> doorVertices;
    std::vector<IPLTriangle> doorTriangles;
    addQuad(doorVertices, doorTriangles, {0, -1.5f, -1.2f}, {0, 1.5f, -1.2f},
            {0, 1.5f, 1.2f}, {0, -1.5f, 1.2f});
    auto staticGeometry = flattenTriangles(vertices, triangles, 0);
    auto doorGeometry = flattenTriangles(doorVertices, doorTriangles, 1);
    IPLMaterial materials[2] = {
        {{0.12f, 0.20f, 0.35f}, 0.18f, {0.08f, 0.05f, 0.03f}},
        {{0.20f, 0.30f, 0.45f}, 0.10f, {0.03f, 0.02f, 0.01f}},
    };
    if (afterglow_obvhs_create(staticGeometry.data(), static_cast<uint32_t>(staticGeometry.size()),
                               doorGeometry.data(), static_cast<uint32_t>(doorGeometry.size()),
                               materials, 2, &gTracer) != 0) return 2;

    IPLSceneSettings sceneSettings{};
    sceneSettings.type = IPL_SCENETYPE_CUSTOM;
    sceneSettings.closestHitCallback = afterglow_obvhs_closest_hit;
    sceneSettings.anyHitCallback = afterglow_obvhs_any_hit;
    sceneSettings.batchedClosestHitCallback = afterglow_obvhs_batched_closest_hit;
    sceneSettings.batchedAnyHitCallback = afterglow_obvhs_batched_any_hit;
    sceneSettings.userData = gTracer;
    if (iplSceneCreate(gContext, &sceneSettings, &gScene) != IPL_STATUS_SUCCESS) return 3;

    IPLSimulationSettings simulationSettings{};
    simulationSettings.flags = static_cast<IPLSimulationFlags>(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS);
    simulationSettings.sceneType = IPL_SCENETYPE_CUSTOM;
    simulationSettings.reflectionType = gReflectionType;
    simulationSettings.maxNumOcclusionSamples = 1;
    simulationSettings.maxNumRays = maxRays;
    simulationSettings.numDiffuseSamples = 32;
    simulationSettings.maxDuration = static_cast<float>(maxDurationMs) / 1000.0f;
    simulationSettings.maxOrder = maxOrder;
    simulationSettings.maxNumSources = sourceCount;
    simulationSettings.numThreads = gSimulationThreads;
    simulationSettings.rayBatchSize = 64;
    simulationSettings.samplingRate = kSampleRate;
    simulationSettings.frameSize = kFrameSize;
    if (iplSimulatorCreate(gContext, &simulationSettings, &gSimulator) != IPL_STATUS_SUCCESS) return 7;
    iplSimulatorSetScene(gSimulator, gScene);
    iplSimulatorCommit(gSimulator);

    const auto flags = static_cast<IPLSimulationFlags>(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS);
    IPLSourceSettings sourceSettings{};
    sourceSettings.flags = flags;
    gSources.resize(static_cast<size_t>(sourceCount), nullptr);
    gSourceInputs.resize(static_cast<size_t>(sourceCount));
    gOutputs.resize(static_cast<size_t>(sourceCount));
    for (int index = 0; index < sourceCount; ++index) {
        if (iplSourceCreate(gSimulator, &sourceSettings, &gSources[index]) != IPL_STATUS_SUCCESS) return 8;
        iplSourceAdd(gSources[index], gSimulator);
        auto& input = gSourceInputs[index];
        input.flags = flags;
        input.directFlags = static_cast<IPLDirectSimulationFlags>(
            IPL_DIRECTSIMULATIONFLAGS_OCCLUSION | IPL_DIRECTSIMULATIONFLAGS_TRANSMISSION);
        input.source = coordinates(-2.0f, 0.0f, static_cast<float>(index % 8) * 0.15f - 0.5f);
        input.occlusionType = IPL_OCCLUSIONTYPE_RAYCAST;
        input.numTransmissionRays = 1;
        input.reverbScale[0] = input.reverbScale[1] = input.reverbScale[2] = 1.0f;
        input.baked = IPL_FALSE;
        iplSourceSetInputs(gSources[index], flags, &input);
    }
    iplSimulatorCommit(gSimulator);

    gSharedInputs.listener = coordinates(2.0f, 0.0f, 0.0f);
    gSharedInputs.numRays = maxRays;
    gSharedInputs.numBounces = maxBounces;
    gSharedInputs.duration = static_cast<float>(maxDurationMs) / 1000.0f;
    gSharedInputs.order = maxOrder;
    gSharedInputs.irradianceMinDistance = 1.0f;
    iplSimulatorSetSharedInputs(gSimulator, flags, &gSharedInputs);

    IPLAudioSettings audioSettings{kSampleRate, kFrameSize};
    IPLReflectionEffectSettings effectSettings{};
    effectSettings.type = gReflectionType;
    effectSettings.irSize = static_cast<int>(std::ceil(simulationSettings.maxDuration * kSampleRate));
    effectSettings.numChannels = (maxOrder + 1) * (maxOrder + 1);
    gEffects.resize(static_cast<size_t>(sourceCount), nullptr);
    for (auto& effect : gEffects) {
        if (iplReflectionEffectCreate(gContext, &audioSettings, &effectSettings, &effect) != IPL_STATUS_SUCCESS) return 9;
    }
    if (iplAudioBufferAllocate(gContext, 1, kFrameSize, &gAudioIn) != IPL_STATUS_SUCCESS) return 10;
    if (iplAudioBufferAllocate(gContext, effectSettings.numChannels, kFrameSize, &gAudioOut) != IPL_STATUS_SUCCESS) return 11;
    IPLHRTFSettings hrtfSettings{};
    hrtfSettings.type = IPL_HRTFTYPE_DEFAULT;
    hrtfSettings.volume = 1.0f;
    hrtfSettings.normType = IPL_HRTFNORMTYPE_NONE;
    if (iplHRTFCreate(gContext, &audioSettings, &hrtfSettings, &gHrtf) != IPL_STATUS_SUCCESS) return 12;
    IPLBinauralEffectSettings binauralSettings{gHrtf};
    gBinauralEffects.resize(static_cast<size_t>(sourceCount), nullptr);
    for (auto& effect : gBinauralEffects)
        if (iplBinauralEffectCreate(gContext, &audioSettings, &binauralSettings, &effect) != IPL_STATUS_SUCCESS) return 13;
    if (iplAudioBufferAllocate(gContext, 2, kFrameSize, &gBinauralOut) != IPL_STATUS_SUCCESS) return 14;
    for (int sample = 0; sample < kFrameSize; ++sample)
        gAudioIn.data[0][sample] = std::sin(static_cast<float>(sample) * 0.0576f) * 0.1f;

    iplSimulatorRunReflections(gSimulator);
    for (size_t index = 0; index < gSources.size(); ++index)
        iplSourceGetOutputs(gSources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &gOutputs[index]);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int dyn_update(float phase) {
    if (!gSimulator || !std::isfinite(phase)) return 101;
    const float doorY = std::sin(phase) * 2.5f;
    afterglow_obvhs_set_door_y(gTracer, doorY);
    for (size_t index = 0; index < gSources.size(); ++index) {
        auto& input = gSourceInputs[index];
        input.source = coordinates(-2.0f + std::sin(phase + static_cast<float>(index) * 0.1f) * 0.5f,
                                   std::cos(phase * 0.7f) * 0.4f,
                                   static_cast<float>(index % 8) * 0.15f - 0.5f);
        iplSourceSetInputs(gSources[index], input.flags, &input);
    }
    gSharedInputs.listener = coordinates(2.0f, std::sin(phase * 0.5f) * 0.4f, 0.0f);
    const auto flags = static_cast<IPLSimulationFlags>(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS);
    iplSimulatorSetSharedInputs(gSimulator, flags, &gSharedInputs);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int dyn_run_reflections(int rays, int bounces, int durationMs, int order) {
    if (!gSimulator || rays < 1 || rays > gMaxRays ||
        bounces < 1 || bounces > gMaxBounces ||
        durationMs < 1 || durationMs > gMaxDurationMs ||
        order < 0 || order > gMaxOrder) return 102;
    gSharedInputs.numRays = rays;
    gSharedInputs.numBounces = bounces;
    gSharedInputs.duration = static_cast<float>(durationMs) / 1000.0f;
    gSharedInputs.order = order;
    iplSimulatorSetSharedInputs(gSimulator, IPL_SIMULATIONFLAGS_REFLECTIONS, &gSharedInputs);
    iplSimulatorRunReflections(gSimulator);
    for (size_t index = 0; index < gSources.size(); ++index)
        iplSourceGetOutputs(gSources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &gOutputs[index]);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int dyn_run_audio(int iterations) {
    if (!gAudioIn.data || iterations < 1 || iterations > 10'000) return 103;
    for (int iteration = 0; iteration < iterations; ++iteration) {
        for (int sample = 0; sample < kFrameSize; ++sample) {
            const float time = static_cast<float>(iteration * kFrameSize + sample);
            gAudioIn.data[0][sample] = std::sin(time * 0.0576f) * 0.1f;
        }
        for (size_t index = 0; index < gEffects.size(); ++index)
            iplReflectionEffectApply(gEffects[index], &gOutputs[index].reflections,
                                     &gAudioIn, &gAudioOut, nullptr);
    }
    gOutputEnergy = 0.0f;
    for (int channel = 0; channel < gAudioOut.numChannels; ++channel)
        for (int sample = 0; sample < gAudioOut.numSamples; ++sample)
            gOutputEnergy += std::fabs(gAudioOut.data[channel][sample]);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int dyn_run_binaural(int iterations) {
    if (!gAudioIn.data || iterations < 1 || iterations > 10'000) return 104;
    for (int iteration = 0; iteration < iterations; ++iteration) {
        for (int sample = 0; sample < kFrameSize; ++sample) {
            const float time = static_cast<float>(iteration * kFrameSize + sample);
            gAudioIn.data[0][sample] = std::sin(time * 0.0576f) * 0.1f;
        }
        for (size_t index = 0; index < gBinauralEffects.size(); ++index) {
            const auto& origin = gSourceInputs[index].source.origin;
            const auto& listener = gSharedInputs.listener.origin;
            float x = origin.x - listener.x;
            float y = origin.y - listener.y;
            float z = origin.z - listener.z;
            const float inverseLength = 1.0f / std::sqrt(x * x + y * y + z * z);
            IPLBinauralEffectParams params{};
            params.direction = {x * inverseLength, y * inverseLength, z * inverseLength};
            params.interpolation = IPL_HRTFINTERPOLATION_NEAREST;
            params.spatialBlend = 1.0f;
            params.hrtf = gHrtf;
            iplBinauralEffectApply(gBinauralEffects[index], &params, &gAudioIn, &gBinauralOut);
        }
    }
    gOutputEnergy = 0.0f;
    for (int channel = 0; channel < gBinauralOut.numChannels; ++channel)
        for (int sample = 0; sample < gBinauralOut.numSamples; ++sample)
            gOutputEnergy += std::fabs(gBinauralOut.data[channel][sample]);
    return 0;
}

EMSCRIPTEN_KEEPALIVE float dyn_get_reverb_low() { return gOutputs.empty() ? 0.0f : gOutputs[0].reflections.reverbTimes[0]; }
EMSCRIPTEN_KEEPALIVE float dyn_get_reverb_mid() { return gOutputs.empty() ? 0.0f : gOutputs[0].reflections.reverbTimes[1]; }
EMSCRIPTEN_KEEPALIVE float dyn_get_reverb_high() { return gOutputs.empty() ? 0.0f : gOutputs[0].reflections.reverbTimes[2]; }
EMSCRIPTEN_KEEPALIVE int dyn_get_ir_valid() { return (!gOutputs.empty() && gOutputs[0].reflections.ir) ? 1 : 0; }
EMSCRIPTEN_KEEPALIVE float dyn_get_output_energy() { return gOutputEnergy; }
EMSCRIPTEN_KEEPALIVE int dyn_get_tracer_nodes() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return static_cast<int>(stats.staticNodeCount + stats.doorNodeCount);
}
EMSCRIPTEN_KEEPALIVE double dyn_get_tracer_build_ms() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return stats.buildMilliseconds;
}
EMSCRIPTEN_KEEPALIVE double dyn_get_tracer_owned_bytes() {
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return static_cast<double>(stats.ownedBytes);
}
EMSCRIPTEN_KEEPALIVE void dyn_shutdown() { releaseAll(); }

}
