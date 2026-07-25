#include <phonon.h>
#include <afterglow_obvhs_tracer.h>
#if defined(__EMSCRIPTEN__)
#include <emscripten/emscripten.h>
#else
#define EMSCRIPTEN_KEEPALIVE
#endif
#include <algorithm>
#include <array>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <vector>

namespace {
constexpr int kSampleRate = 48000;
constexpr int kFrameSize = 128;
constexpr int kMaxEngineVoiceCount = 128;
constexpr int kMaxEngineReflectionVoiceCount = 64;
constexpr int kMaxResidentSoundCount = 64;
constexpr std::array<char, 8> kAcousticMagic{'A', 'G', 'B', 'I', 'S', 'T', '1', '\0'};

struct AcousticSceneHeader {
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
static_assert(sizeof(AcousticSceneHeader) == 72);

IPLContext gContext = nullptr;
IPLScene gScene = nullptr;
void* gTracer = nullptr;
#if !defined(__EMSCRIPTEN__)
IPLEmbreeDevice gEmbreeDevice = nullptr;
IPLStaticMesh gStaticMesh = nullptr;
#endif
IPLSimulator gSimulator = nullptr;
std::vector<IPLSource> gSources;
std::vector<IPLSimulationInputs> gSourceInputs;
std::vector<IPLSimulationOutputs> gOutputs;

// Steam's convolution IR is already internally triple-buffered, but the
// scalar simulation outputs and source/listener transforms are ordinary
// memory. Publish those values through a second fixed triple buffer so the
// simulation worker never races the real-time audio callback.
struct EngineSimulationSnapshot {
    std::array<IPLSimulationOutputs, kMaxEngineVoiceCount> outputs{};
    std::array<IPLVector3, kMaxEngineVoiceCount> sourceOrigins{};
    IPLCoordinateSpace3 listener{};
};
std::array<EngineSimulationSnapshot, 3> gSimulationSnapshots{};
int gSnapshotFrontIndex = 0; // AudioWorklet thread only.
std::atomic<int> gSnapshotMiddleIndex{1};
int gSnapshotBackIndex = 2; // Simulation worker only.
std::atomic<bool> gSnapshotDirty{false};

std::vector<IPLReflectionEffect> gEffects;
IPLHRTF gHrtf = nullptr;
std::vector<IPLBinauralEffect> gBinauralEffects;
std::vector<IPLDirectEffect> gDirectEffects;
IPLAmbisonicsDecodeEffect gAmbisonicsDecode = nullptr;
IPLSimulationSharedInputs gSharedInputs{};
IPLAudioBuffer gAudioIn{};
IPLAudioBuffer gAudioOut{};
IPLAudioBuffer gBinauralOut{};
IPLAudioBuffer gDirectOut{};
IPLAudioBuffer gReflectionMix{};
IPLAudioBuffer gWetStereo{};
std::array<float, kFrameSize * 2> gEnginePcm{};
std::array<std::array<float, kFrameSize>, 4> gProducerPcm{};
uint64_t gEngineSampleClock = 0;
uint64_t gEngineLastImpulseSample = 0;
std::atomic<uint64_t> gEngineSampleClockPublished{0};
std::atomic<uint64_t> gEngineLastImpulseSamplePublished{0};
std::atomic<uint32_t> gEnginePeakBits{0};
std::atomic<uint32_t> gOutputEnergyBits{0};
int gReflectionEffectLimit = 128;
std::atomic<int> gActiveReflectionEffectLimit{0};
int gSpatialDirectVoiceLimit = 0;

enum EngineVoiceMode : int {
    kVoiceInactive = 0,
    kVoiceWorldPhysical = 1,
    kVoiceTwoD = 2,
    kVoiceSpatialOnly = 3,
    kVoiceListenerRelative = 4,
};
struct EngineVoiceControl {
    int mode = kVoiceInactive;
    uint32_t sound = 0;
    IPLVector3 position{};
    float previousGain = 0.0f;
    float gain = 0.0f;
    uint64_t cursor = 0;
};
struct EngineResidentSound {
    uint32_t handle = 0;
    const float* samples = nullptr;
    uint32_t frames = 0;
    uint32_t channels = 0;
    bool looped = false;
};
std::array<EngineVoiceControl, kMaxEngineVoiceCount> gVoiceControls{};
std::array<EngineResidentSound, kMaxResidentSoundCount> gResidentSounds{};
bool gVoiceControlEnabled = false;
IPLReflectionEffectType gReflectionType = IPL_REFLECTIONEFFECTTYPE_PARAMETRIC;
int gMaxRays = 0;
int gMaxBounces = 0;
int gMaxDurationMs = 0;
int gMaxOrder = 0;
int gSimulationThreads = 2;
float gOutputEnergy = 0.0f;
float gEnginePeak = 0.0f;
uint32_t gAcousticVertices = 0;
uint32_t gAcousticTriangles = 0;

uint32_t floatBits(float value) {
    uint32_t bits = 0;
    static_assert(sizeof(bits) == sizeof(value));
    std::memcpy(&bits, &value, sizeof(bits));
    return bits;
}

float bitsFloat(uint32_t bits) {
    float value = 0.0f;
    std::memcpy(&value, &bits, sizeof(value));
    return value;
}

void publishSimulationSnapshot() {
    auto& snapshot = gSimulationSnapshots[static_cast<size_t>(gSnapshotBackIndex)];
    const size_t count = std::min(gOutputs.size(), snapshot.outputs.size());
    for (size_t index = 0; index < count; ++index) {
        snapshot.outputs[index] = gOutputs[index];
        snapshot.sourceOrigins[index] = gSourceInputs[index].source.origin;
    }
    snapshot.listener = gSharedInputs.listener;
    const int previousMiddle = gSnapshotMiddleIndex.exchange(
        gSnapshotBackIndex, std::memory_order_acq_rel);
    gSnapshotBackIndex = previousMiddle;
    gSnapshotDirty.store(true, std::memory_order_release);
}

const EngineSimulationSnapshot& consumeSimulationSnapshot() {
    if (gSnapshotDirty.exchange(false, std::memory_order_acquire)) {
        gSnapshotFrontIndex = gSnapshotMiddleIndex.exchange(
            gSnapshotFrontIndex, std::memory_order_acq_rel);
    }
    return gSimulationSnapshots[static_cast<size_t>(gSnapshotFrontIndex)];
}

const EngineResidentSound* residentSound(uint32_t handle) {
    const uint32_t encodedIndex = handle & 0xffu;
    if (encodedIndex == 0 || encodedIndex > gResidentSounds.size()) return nullptr;
    const auto& sound = gResidentSounds[encodedIndex - 1];
    return sound.handle == handle ? &sound : nullptr;
}

float residentSample(const EngineResidentSound& sound, uint64_t frame, uint32_t channel) {
    if (sound.looped) frame %= sound.frames;
    else if (frame >= sound.frames) return 0.0f;
    channel = std::min(channel, sound.channels - 1);
    return sound.samples[frame * sound.channels + channel];
}

bool exactAcousticGeometrySize(const AcousticSceneHeader& header, uint32_t byteCount) {
    const uint64_t positions = static_cast<uint64_t>(header.vertexCount) * 3 * sizeof(float);
    const uint64_t indices = static_cast<uint64_t>(header.triangleCount) * 3 * sizeof(uint32_t);
    const uint64_t materials = header.triangleCount;
    return sizeof(AcousticSceneHeader) + positions + indices + materials == byteCount;
}

std::array<IPLMaterial, 6> acousticMaterials() {
    return {{
        {{0.20f, 0.30f, 0.40f}, 0.20f, {0.05f, 0.03f, 0.02f}},
        {{0.10f, 0.05f, 0.02f}, 0.05f, {0.10f, 0.05f, 0.02f}},
        {{0.40f, 0.60f, 0.70f}, 0.50f, {0.02f, 0.01f, 0.01f}},
        {{0.15f, 0.11f, 0.10f}, 0.30f, {0.03f, 0.02f, 0.01f}},
        {{0.05f, 0.04f, 0.03f}, 0.10f, {0.01f, 0.01f, 0.01f}},
        {{0.10f, 0.05f, 0.03f}, 0.20f, {0.02f, 0.01f, 0.01f}},
    }};
}

IPLCoordinateSpace3 coordinates(float x, float y, float z) {
    IPLCoordinateSpace3 value{};
    value.ahead = {0.0f, 0.0f, -1.0f};
    value.up = {0.0f, 1.0f, 0.0f};
    value.right = {1.0f, 0.0f, 0.0f};
    value.origin = {x, y, z};
    return value;
}

void clearAudio(IPLAudioBuffer& buffer) {
    for (int channel = 0; channel < buffer.numChannels; ++channel)
        for (int sample = 0; sample < buffer.numSamples; ++sample)
            buffer.data[channel][sample] = 0.0f;
}

void releaseAll() {
    if (gWetStereo.data) iplAudioBufferFree(gContext, &gWetStereo);
    if (gReflectionMix.data) iplAudioBufferFree(gContext, &gReflectionMix);
    if (gDirectOut.data) iplAudioBufferFree(gContext, &gDirectOut);
    if (gAmbisonicsDecode) iplAmbisonicsDecodeEffectRelease(&gAmbisonicsDecode);
    for (auto& effect : gDirectEffects) if (effect) iplDirectEffectRelease(&effect);
    gDirectEffects.clear();
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
    for (auto& snapshot : gSimulationSnapshots) snapshot = {};
    gSnapshotFrontIndex = 0;
    gSnapshotMiddleIndex.store(1, std::memory_order_relaxed);
    gSnapshotBackIndex = 2;
    gSnapshotDirty.store(false, std::memory_order_relaxed);
    if (gSimulator) iplSimulatorRelease(&gSimulator);
#if !defined(__EMSCRIPTEN__)
    if (gStaticMesh) {
        iplStaticMeshRemove(gStaticMesh, gScene);
        iplStaticMeshRelease(&gStaticMesh);
    }
#endif
    if (gScene) iplSceneRelease(&gScene);
#if !defined(__EMSCRIPTEN__)
    if (gEmbreeDevice) iplEmbreeDeviceRelease(&gEmbreeDevice);
#endif
    if (gTracer) {
        afterglow_obvhs_destroy(gTracer);
        gTracer = nullptr;
    }
    if (gContext) iplContextRelease(&gContext);
    gAudioIn = {};
    gAudioOut = {};
    gBinauralOut = {};
    gDirectOut = {};
    gReflectionMix = {};
    gWetStereo = {};
    gEnginePcm.fill(0.0f);
    for (auto& producer : gProducerPcm) producer.fill(0.0f);
    gEngineSampleClock = 0;
    gEngineLastImpulseSample = 0;
    gEngineSampleClockPublished.store(0, std::memory_order_relaxed);
    gEngineLastImpulseSamplePublished.store(0, std::memory_order_relaxed);
    gEnginePeak = 0.0f;
    gOutputEnergy = 0.0f;
    gEnginePeakBits.store(0, std::memory_order_relaxed);
    gOutputEnergyBits.store(0, std::memory_order_relaxed);
    gSpatialDirectVoiceLimit = 0;
    gActiveReflectionEffectLimit.store(0, std::memory_order_relaxed);
    gVoiceControls = {};
    gResidentSounds = {};
    gVoiceControlEnabled = false;
    gAcousticVertices = 0;
    gAcousticTriangles = 0;
}

#if defined(__EMSCRIPTEN__)
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
#endif

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
         reflectionType != IPL_REFLECTIONEFFECTTYPE_PARAMETRIC &&
         reflectionType != IPL_REFLECTIONEFFECTTYPE_HYBRID) ||
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
    IPLMaterial materials[2] = {
        {{0.12f, 0.20f, 0.35f}, 0.18f, {0.08f, 0.05f, 0.03f}},
        {{0.20f, 0.30f, 0.45f}, 0.10f, {0.03f, 0.02f, 0.01f}},
    };

    IPLSceneSettings sceneSettings{};
#if defined(__EMSCRIPTEN__)
    auto staticGeometry = flattenTriangles(vertices, triangles, 0);
    auto doorGeometry = flattenTriangles(doorVertices, doorTriangles, 1);
    if (afterglow_obvhs_create(staticGeometry.data(), static_cast<uint32_t>(staticGeometry.size()),
                               doorGeometry.data(), static_cast<uint32_t>(doorGeometry.size()),
                               materials, 2, &gTracer) != 0) return 2;
    sceneSettings.type = IPL_SCENETYPE_CUSTOM;
    sceneSettings.closestHitCallback = afterglow_obvhs_closest_hit;
    sceneSettings.anyHitCallback = afterglow_obvhs_any_hit;
    sceneSettings.batchedClosestHitCallback = afterglow_obvhs_batched_closest_hit;
    sceneSettings.batchedAnyHitCallback = afterglow_obvhs_batched_any_hit;
    sceneSettings.userData = gTracer;
#else
    IPLEmbreeDeviceSettings embreeSettings{};
    if (iplEmbreeDeviceCreate(gContext, &embreeSettings, &gEmbreeDevice) != IPL_STATUS_SUCCESS)
        return 2;
    sceneSettings.type = IPL_SCENETYPE_EMBREE;
    sceneSettings.embreeDevice = gEmbreeDevice;
#endif
    if (iplSceneCreate(gContext, &sceneSettings, &gScene) != IPL_STATUS_SUCCESS) return 3;

#if !defined(__EMSCRIPTEN__)
    const IPLint32 doorBase = static_cast<IPLint32>(vertices.size());
    vertices.insert(vertices.end(), doorVertices.begin(), doorVertices.end());
    for (const auto& triangle : doorTriangles) {
        triangles.push_back({triangle.indices[0] + doorBase,
                             triangle.indices[1] + doorBase,
                             triangle.indices[2] + doorBase});
    }
    std::vector<IPLint32> materialIndices(triangles.size(), 0);
    for (size_t index = triangles.size() - doorTriangles.size(); index < triangles.size(); ++index)
        materialIndices[index] = 1;
    IPLStaticMeshSettings meshSettings{};
    meshSettings.numVertices = static_cast<IPLint32>(vertices.size());
    meshSettings.numTriangles = static_cast<IPLint32>(triangles.size());
    meshSettings.numMaterials = 2;
    meshSettings.vertices = vertices.data();
    meshSettings.triangles = triangles.data();
    meshSettings.materialIndices = materialIndices.data();
    meshSettings.materials = materials;
    if (iplStaticMeshCreate(gScene, &meshSettings, &gStaticMesh) != IPL_STATUS_SUCCESS) return 4;
    iplStaticMeshAdd(gStaticMesh, gScene);
    iplSceneCommit(gScene);
#endif

    IPLSimulationSettings simulationSettings{};
    simulationSettings.flags = static_cast<IPLSimulationFlags>(IPL_SIMULATIONFLAGS_DIRECT | IPL_SIMULATIONFLAGS_REFLECTIONS);
#if defined(__EMSCRIPTEN__)
    simulationSettings.sceneType = IPL_SCENETYPE_CUSTOM;
#else
    simulationSettings.sceneType = IPL_SCENETYPE_EMBREE;
#endif
    simulationSettings.reflectionType = gReflectionType;
    simulationSettings.maxNumOcclusionSamples = 1;
    simulationSettings.maxNumRays = maxRays;
    simulationSettings.numDiffuseSamples = 32;
    simulationSettings.maxDuration = static_cast<float>(maxDurationMs) / 1000.0f;
    simulationSettings.maxOrder = maxOrder;
    simulationSettings.maxNumSources = sourceCount;
    simulationSettings.numThreads = gSimulationThreads;
#if defined(__EMSCRIPTEN__)
    simulationSettings.rayBatchSize = 64;
#else
    simulationSettings.rayBatchSize = 1;
#endif
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
        input.hybridReverbTransitionTime = 0.032f;
        input.hybridReverbOverlapPercent = 0.25f;
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
    gEffects.resize(static_cast<size_t>(std::min(sourceCount, gReflectionEffectLimit)), nullptr);
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
#if defined(__EMSCRIPTEN__)
    const float doorY = std::sin(phase) * 2.5f;
    afterglow_obvhs_set_door_y(gTracer, doorY);
#endif
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
#if defined(__EMSCRIPTEN__)
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return static_cast<int>(stats.staticNodeCount + stats.doorNodeCount);
#else
    return 0;
#endif
}
EMSCRIPTEN_KEEPALIVE double dyn_get_tracer_build_ms() {
#if defined(__EMSCRIPTEN__)
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return stats.buildMilliseconds;
#else
    return 0.0;
#endif
}
EMSCRIPTEN_KEEPALIVE double dyn_get_tracer_owned_bytes() {
#if defined(__EMSCRIPTEN__)
    AfterglowObvhsStats stats{};
    afterglow_obvhs_get_stats(gTracer, &stats);
    return static_cast<double>(stats.ownedBytes);
#else
    return 0.0;
#endif
}
EMSCRIPTEN_KEEPALIVE int dyn_get_simulation_threads() { return gSimulationThreads; }
EMSCRIPTEN_KEEPALIVE int dyn_get_tracer_lanes() {
#if defined(__EMSCRIPTEN__)
    return static_cast<int>(afterglow_obvhs_traversal_lanes());
#else
    return 0;
#endif
}
EMSCRIPTEN_KEEPALIVE void dyn_shutdown() { releaseAll(); }

// Thin C ABI used by the unified Rust #[rpc] worker. Rust owns lifecycle,
// scheduling, telemetry, ring publication, and failure policy; this layer owns
// only Steam Audio objects that must be created and called through phonon.h.
int afterglow_steam_audio_init(uint32_t triangleCount, uint32_t voices,
                                 uint32_t reflectionVoices, uint32_t rays,
                                 uint32_t bounces, uint32_t durationMs,
                                 uint32_t order) {
    if (voices == 0 || voices > kMaxEngineVoiceCount || reflectionVoices == 0 ||
        reflectionVoices > kMaxEngineReflectionVoiceCount || reflectionVoices > voices)
        return 200;
    gReflectionEffectLimit = static_cast<int>(reflectionVoices);
    const int initialized = dyn_init(static_cast<int>(triangleCount), static_cast<int>(voices),
                                     static_cast<int>(rays), static_cast<int>(bounces),
                                     IPL_REFLECTIONEFFECTTYPE_HYBRID,
                                     static_cast<int>(durationMs), static_cast<int>(order));
    if (initialized != 0) {
        releaseAll();
        return initialized;
    }
    // A world-physical voice is all-or-nothing: every spatialized engine voice
    // also receives one reflection effect. Target profiles reduce this count
    // instead of silently rendering partial acoustics.
    gSpatialDirectVoiceLimit = static_cast<int>(reflectionVoices);
    gActiveReflectionEffectLimit.store(
        static_cast<int>(reflectionVoices), std::memory_order_relaxed);

    IPLAudioSettings audioSettings{kSampleRate, kFrameSize};
    if (iplAudioBufferAllocate(gContext, gAudioOut.numChannels, kFrameSize,
                               &gReflectionMix) != IPL_STATUS_SUCCESS) {
        releaseAll();
        return 201;
    }
    if (iplAudioBufferAllocate(gContext, 2, kFrameSize, &gWetStereo) != IPL_STATUS_SUCCESS) {
        releaseAll();
        return 202;
    }
    IPLAmbisonicsDecodeEffectSettings decodeSettings{};
    decodeSettings.speakerLayout.type = IPL_SPEAKERLAYOUTTYPE_STEREO;
    decodeSettings.hrtf = gHrtf;
    decodeSettings.maxOrder = static_cast<int>(order);
    if (iplAmbisonicsDecodeEffectCreate(gContext, &audioSettings, &decodeSettings,
                                        &gAmbisonicsDecode) != IPL_STATUS_SUCCESS) {
        releaseAll();
        return 203;
    }
    iplSimulatorRunDirect(gSimulator);
    const auto flags = static_cast<IPLSimulationFlags>(IPL_SIMULATIONFLAGS_DIRECT |
                                                        IPL_SIMULATIONFLAGS_REFLECTIONS);
    for (size_t index = 0; index < gSources.size(); ++index)
        iplSourceGetOutputs(gSources[index], flags, &gOutputs[index]);
    publishSimulationSnapshot();
    return 0;
}

int afterglow_steam_audio_set_active_reflection_voices(uint32_t voices) {
    if (voices > gEffects.size()) return 204;
    gActiveReflectionEffectLimit.store(static_cast<int>(voices), std::memory_order_release);
    return 0;
}

int afterglow_steam_audio_update_motion(float phase) { return dyn_update(phase); }

int afterglow_steam_audio_run_direct_simulation() {
    if (!gSimulator) return 205;
    iplSimulatorRunDirect(gSimulator);
    for (size_t index = 0; index < gSources.size(); ++index)
        iplSourceGetOutputs(gSources[index], IPL_SIMULATIONFLAGS_DIRECT, &gOutputs[index]);
    publishSimulationSnapshot();
    return 0;
}

int afterglow_steam_audio_run_reflection_simulation() {
    if (!gSimulator) return 205;
    iplSimulatorRunReflections(gSimulator);
    for (size_t index = 0; index < gSources.size(); ++index)
        iplSourceGetOutputs(gSources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &gOutputs[index]);
    publishSimulationSnapshot();
    return 0;
}

int afterglow_steam_audio_run_simulation() {
    const int direct = afterglow_steam_audio_run_direct_simulation();
    if (direct != 0) return direct;
    return afterglow_steam_audio_run_reflection_simulation();
}

int afterglow_steam_audio_load_acoustic_scene(const uint8_t* bytes, uint32_t byteCount) {
    if (!gSimulator || !bytes || byteCount < sizeof(AcousticSceneHeader)) return 210;
    AcousticSceneHeader header{};
    std::memcpy(&header, bytes, sizeof(header));
    if (header.magic != kAcousticMagic || header.version != 1 ||
        header.vertexCount == 0 || header.triangleCount == 0 ||
        header.vertexCount > static_cast<uint32_t>(std::numeric_limits<IPLint32>::max()) ||
        header.triangleCount > static_cast<uint32_t>(std::numeric_limits<IPLint32>::max()) ||
        header.materialCount != 6 || !exactAcousticGeometrySize(header, byteCount))
        return 211;
    for (float value : header.listener) if (!std::isfinite(value)) return 211;
    for (float value : header.source) if (!std::isfinite(value)) return 211;
    const auto* vertices = reinterpret_cast<const IPLVector3*>(bytes + sizeof(header));
    const auto* indices = reinterpret_cast<const uint32_t*>(vertices + header.vertexCount);
    const auto* materialBytes = reinterpret_cast<const uint8_t*>(
        indices + static_cast<uint64_t>(header.triangleCount) * 3);
    for (uint64_t index = 0; index < static_cast<uint64_t>(header.triangleCount) * 3; ++index)
        if (indices[index] >= header.vertexCount) return 212;
    for (uint32_t index = 0; index < header.triangleCount; ++index)
        if (materialBytes[index] >= header.materialCount) return 213;

    IPLScene newScene = nullptr;
#if defined(__EMSCRIPTEN__)
    void* newTracer = nullptr;
    auto materials = acousticMaterials();
    if (afterglow_obvhs_create_indexed(vertices, header.vertexCount, indices,
                                       materialBytes, header.triangleCount,
                                       materials.data(), materials.size(), &newTracer) != 0)
        return 214;
    IPLSceneSettings sceneSettings{};
    sceneSettings.type = IPL_SCENETYPE_CUSTOM;
    sceneSettings.closestHitCallback = afterglow_obvhs_closest_hit;
    sceneSettings.anyHitCallback = afterglow_obvhs_any_hit;
    sceneSettings.batchedClosestHitCallback = afterglow_obvhs_batched_closest_hit;
    sceneSettings.batchedAnyHitCallback = afterglow_obvhs_batched_any_hit;
    sceneSettings.userData = newTracer;
    if (iplSceneCreate(gContext, &sceneSettings, &newScene) != IPL_STATUS_SUCCESS) {
        afterglow_obvhs_destroy(newTracer);
        return 215;
    }
#else
    IPLStaticMesh newMesh = nullptr;
    IPLSceneSettings sceneSettings{};
    sceneSettings.type = IPL_SCENETYPE_EMBREE;
    sceneSettings.embreeDevice = gEmbreeDevice;
    if (iplSceneCreate(gContext, &sceneSettings, &newScene) != IPL_STATUS_SUCCESS) return 215;
    std::vector<IPLTriangle> triangles(header.triangleCount);
    std::vector<IPLint32> materialIndices(header.triangleCount);
    for (uint32_t index = 0; index < header.triangleCount; ++index) {
        triangles[index] = {static_cast<IPLint32>(indices[index * 3]),
                            static_cast<IPLint32>(indices[index * 3 + 1]),
                            static_cast<IPLint32>(indices[index * 3 + 2])};
        materialIndices[index] = materialBytes[index];
    }
    auto materials = acousticMaterials();
    IPLStaticMeshSettings meshSettings{};
    meshSettings.numVertices = static_cast<IPLint32>(header.vertexCount);
    meshSettings.numTriangles = static_cast<IPLint32>(header.triangleCount);
    meshSettings.numMaterials = static_cast<IPLint32>(materials.size());
    meshSettings.vertices = const_cast<IPLVector3*>(vertices);
    meshSettings.triangles = triangles.data();
    meshSettings.materialIndices = materialIndices.data();
    meshSettings.materials = materials.data();
    if (iplStaticMeshCreate(newScene, &meshSettings, &newMesh) != IPL_STATUS_SUCCESS) {
        iplSceneRelease(&newScene);
        return 216;
    }
    iplStaticMeshAdd(newMesh, newScene);
    iplSceneCommit(newScene);
#endif

    iplSimulatorSetScene(gSimulator, newScene);
    iplSimulatorCommit(gSimulator);
#if defined(__EMSCRIPTEN__)
    if (gScene) iplSceneRelease(&gScene);
    if (gTracer) afterglow_obvhs_destroy(gTracer);
    gTracer = newTracer;
#else
    if (gStaticMesh) {
        iplStaticMeshRemove(gStaticMesh, gScene);
        iplStaticMeshRelease(&gStaticMesh);
    }
    if (gScene) iplSceneRelease(&gScene);
    gStaticMesh = newMesh;
#endif
    gScene = newScene;
    gAcousticVertices = header.vertexCount;
    gAcousticTriangles = header.triangleCount;
    gSharedInputs.listener = coordinates(header.listener[0], header.listener[1], header.listener[2]);
    for (size_t index = 0; index < gSourceInputs.size(); ++index) {
        auto& input = gSourceInputs[index];
        input.source = coordinates(header.source[0] + static_cast<float>(index % 4) * 0.08f,
                                   header.source[1],
                                   header.source[2] + static_cast<float>(index / 4) * 0.08f);
        iplSourceSetInputs(gSources[index], input.flags, &input);
    }
    iplSimulatorCommit(gSimulator);
    publishSimulationSnapshot();
    return 0;
}

uint32_t afterglow_steam_audio_acoustic_vertices() { return gAcousticVertices; }
uint32_t afterglow_steam_audio_acoustic_triangles() { return gAcousticTriangles; }

int afterglow_steam_audio_register_sound(uint32_t handle, const float* samples,
                                          uint32_t frames, uint32_t channels,
                                          int looped) {
    const uint32_t encodedIndex = handle & 0xffu;
    const uint32_t generation = handle >> 8u;
    if (encodedIndex == 0 || encodedIndex > gResidentSounds.size() || generation == 0 ||
        !samples || frames == 0 || (channels != 1 && channels != 2))
        return 208;
    gResidentSounds[encodedIndex - 1] = {handle, samples, frames, channels, looped != 0};
    return 0;
}

int afterglow_steam_audio_unregister_sound(uint32_t handle) {
    const uint32_t encodedIndex = handle & 0xffu;
    if (encodedIndex == 0 || encodedIndex > gResidentSounds.size() ||
        gResidentSounds[encodedIndex - 1].handle != handle)
        return 209;
    gResidentSounds[encodedIndex - 1] = {};
    return 0;
}

int afterglow_steam_audio_set_voice(uint32_t index, int mode, uint32_t sound,
                                     float x, float y, float z, float gain,
                                     uint64_t cursor) {
    if (index >= gSources.size() || mode < kVoiceInactive ||
        mode > kVoiceListenerRelative || !std::isfinite(x) || !std::isfinite(y) ||
        !std::isfinite(z) || !std::isfinite(gain) || gain < 0.0f || gain > 1.0f)
        return 207;
    auto& control = gVoiceControls[index];
    control.mode = mode;
    control.sound = sound;
    control.position = {x, y, z};
    control.gain = gain;
    control.cursor = cursor;
    gVoiceControlEnabled = true;
    if (mode == kVoiceWorldPhysical || mode == kVoiceSpatialOnly) {
        gSourceInputs[index].source.origin = control.position;
        iplSourceSetInputs(gSources[index], gSourceInputs[index].flags, &gSourceInputs[index]);
    }
    return 0;
}

int afterglow_steam_audio_render_quantum() {
    if (!gAudioIn.data || !gReflectionMix.data || !gWetStereo.data ||
        !gAmbisonicsDecode || gSources.empty() ||
        gSources.size() > static_cast<size_t>(kMaxEngineVoiceCount))
        return 206;
    gEnginePcm.fill(0.0f);
    gOutputEnergy = 0.0f;
    gEnginePeak = 0.0f;
    clearAudio(gReflectionMix);
    const auto& simulation = consumeSimulationSnapshot();
    IPLAmbisonicsDecodeEffectParams decodeParams{};
    decodeParams.order = gMaxOrder;
    decodeParams.hrtf = gHrtf;
    decodeParams.orientation = simulation.listener;
    decodeParams.binaural = IPL_TRUE;

    // Produce each source family once on the shared clock. Thirty-two voices
    // consume each family; producer generation must not repeat transcendental
    // work per voice.
    for (int sample = 0; sample < kFrameSize; ++sample) {
        const uint64_t absolute = gEngineSampleClock + static_cast<uint64_t>(sample);
        const float time = static_cast<float>(absolute);
        gProducerPcm[0][sample] = std::sin(time * 0.012f) * 0.0125f;
        gProducerPcm[1][sample] =
            (static_cast<float>(absolute % 113u) / 56.5f - 1.0f) * 0.0125f;
        uint32_t bits = static_cast<uint32_t>(absolute) ^ 0x9e3779b9u;
        bits ^= bits << 13; bits ^= bits >> 17; bits ^= bits << 5;
        gProducerPcm[2][sample] =
            (static_cast<float>(bits & 0xffffu) / 32767.5f - 1.0f) * 0.0125f;
        float procedural = std::sin(time * 0.021f) * 0.0125f;
        if (absolute % kSampleRate == 0) {
            procedural = 0.0125f;
            gEngineLastImpulseSample = absolute;
        }
        gProducerPcm[3][sample] = procedural;
    }

    for (size_t index = 0; index < gSources.size(); ++index) {
        auto& control = gVoiceControls[index];
        if (gVoiceControlEnabled && control.mode == kVoiceInactive) {
            control.previousGain = 0.0f;
            continue;
        }
        const size_t producerKind = gVoiceControlEnabled ? control.sound & 3u : index & 3u;
        const EngineResidentSound* resident = gVoiceControlEnabled
            ? residentSound(control.sound)
            : nullptr;
        const bool worldPhysical = gVoiceControlEnabled
            ? control.mode == kVoiceWorldPhysical
            : index < static_cast<size_t>(gSpatialDirectVoiceLimit);
        const bool spatialized = gVoiceControlEnabled
            ? (worldPhysical || control.mode == kVoiceSpatialOnly ||
               control.mode == kVoiceListenerRelative)
            : index < static_cast<size_t>(gSpatialDirectVoiceLimit);
        const auto& direct = simulation.outputs[index].direct;
        const float transmitted = (direct.transmission[0] + direct.transmission[1] +
                                   direct.transmission[2]) / 3.0f;
        const float directGain = worldPhysical
            ? direct.occlusion + (1.0f - direct.occlusion) * transmitted
            : 1.0f;
        for (int sample = 0; sample < kFrameSize; ++sample) {
            const float gain = gVoiceControlEnabled
                ? control.previousGain + (control.gain - control.previousGain) *
                    static_cast<float>(sample + 1) / static_cast<float>(kFrameSize)
                : 1.0f;
            const float source = resident
                ? (residentSample(*resident, control.cursor + sample, 0) +
                   residentSample(*resident, control.cursor + sample,
                                  resident->channels - 1)) * 0.5f
                : gProducerPcm[producerKind][sample];
            gAudioIn.data[0][sample] = source * directGain * gain;
        }
        if (spatialized) {
            IPLVector3 origin = gVoiceControlEnabled
                ? control.position
                : simulation.sourceOrigins[index];
            const auto& listener = simulation.listener.origin;
            if (gVoiceControlEnabled && control.mode == kVoiceListenerRelative) {
                origin.x += listener.x;
                origin.y += listener.y;
                origin.z += listener.z;
            }
            float x = origin.x - listener.x;
            float y = origin.y - listener.y;
            float z = origin.z - listener.z;
            const float inverseLength = 1.0f / std::sqrt(std::max(x * x + y * y + z * z, 1.0e-8f));
            IPLBinauralEffectParams binauralParams{};
            binauralParams.direction = {x * inverseLength, y * inverseLength, z * inverseLength};
            binauralParams.interpolation = IPL_HRTFINTERPOLATION_NEAREST;
            binauralParams.spatialBlend = 1.0f;
            binauralParams.hrtf = gHrtf;
            clearAudio(gBinauralOut);
            iplBinauralEffectApply(gBinauralEffects[index], &binauralParams,
                                   &gAudioIn, &gBinauralOut);
            for (int sample = 0; sample < kFrameSize; ++sample) {
                gEnginePcm[2 * sample] += gBinauralOut.data[0][sample];
                gEnginePcm[2 * sample + 1] += gBinauralOut.data[1][sample];
            }
        } else {
            for (int sample = 0; sample < kFrameSize; ++sample) {
                if (resident && resident->channels == 2) {
                    const float gain = control.previousGain +
                        (control.gain - control.previousGain) *
                        static_cast<float>(sample + 1) / static_cast<float>(kFrameSize);
                    gEnginePcm[2 * sample] +=
                        residentSample(*resident, control.cursor + sample, 0) * gain;
                    gEnginePcm[2 * sample + 1] +=
                        residentSample(*resident, control.cursor + sample, 1) * gain;
                } else {
                    gEnginePcm[2 * sample] += gAudioIn.data[0][sample];
                    gEnginePcm[2 * sample + 1] += gAudioIn.data[0][sample];
                }
            }
        }

        if (worldPhysical && index < gEffects.size() &&
            index < static_cast<size_t>(gActiveReflectionEffectLimit.load(
                std::memory_order_acquire))) {
            clearAudio(gAudioOut);
            auto reflectionParams = simulation.outputs[index].reflections;
            reflectionParams.irSize = std::min(reflectionParams.irSize,
                                               static_cast<int>(0.032f * kSampleRate));
            iplReflectionEffectApply(gEffects[index], &reflectionParams,
                                     &gAudioIn, &gAudioOut, nullptr);
            for (int channel = 0; channel < gAudioOut.numChannels; ++channel)
                for (int sample = 0; sample < kFrameSize; ++sample)
                    gReflectionMix.data[channel][sample] += gAudioOut.data[channel][sample];
        }
        control.previousGain = control.gain;
    }

    // Decode the summed reflected sound field once. Decoding every source
    // separately is equivalent only before summation and wastes 63 decodes.
    clearAudio(gWetStereo);
    iplAmbisonicsDecodeEffectApply(gAmbisonicsDecode, &decodeParams,
                                   &gReflectionMix, &gWetStereo);
    for (int sample = 0; sample < kFrameSize; ++sample) {
        gEnginePcm[2 * sample] += gWetStereo.data[0][sample];
        gEnginePcm[2 * sample + 1] += gWetStereo.data[1][sample];
    }

    for (float& sample : gEnginePcm) {
        sample = std::clamp(sample * 0.35f, -1.0f, 1.0f);
        gOutputEnergy += std::fabs(sample);
        gEnginePeak = std::max(gEnginePeak, std::fabs(sample));
    }
    gEngineSampleClock += kFrameSize;
    gEngineSampleClockPublished.store(gEngineSampleClock, std::memory_order_release);
    gEngineLastImpulseSamplePublished.store(gEngineLastImpulseSample, std::memory_order_release);
    gOutputEnergyBits.store(floatBits(gOutputEnergy), std::memory_order_release);
    gEnginePeakBits.store(floatBits(gEnginePeak), std::memory_order_release);
    return 0;
}

const float* afterglow_steam_audio_pcm_ptr() { return gEnginePcm.data(); }
uint64_t afterglow_steam_audio_sample_clock() {
    return gEngineSampleClockPublished.load(std::memory_order_acquire);
}
float afterglow_steam_audio_output_energy() {
    return bitsFloat(gOutputEnergyBits.load(std::memory_order_acquire));
}
float afterglow_steam_audio_output_peak() {
    return bitsFloat(gEnginePeakBits.load(std::memory_order_acquire));
}
uint32_t afterglow_steam_audio_active_reflection_voices() {
    return static_cast<uint32_t>(gActiveReflectionEffectLimit.load(std::memory_order_acquire));
}
uint64_t afterglow_steam_audio_last_impulse_sample() {
    return gEngineLastImpulseSamplePublished.load(std::memory_order_acquire);
}
void afterglow_steam_audio_shutdown() { releaseAll(); }

}
