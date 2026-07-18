#include <phonon.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

namespace {
using Clock = std::chrono::steady_clock;
constexpr std::array<char, 8> kMagic{'A', 'G', 'B', 'I', 'S', 'T', '1', '\0'};
constexpr int kSources = 64;
constexpr int kSamples = 30;

struct Header {
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

struct Geometry {
    Header header{};
    std::vector<IPLVector3> vertices;
    std::vector<IPLTriangle> triangles;
    std::vector<IPLint32> materialIndices;
};

struct Summary {
    double mean;
    double p50;
    double p90;
    double p99;
    double max;
};

template <typename Function>
double elapsedMs(Function&& function) {
    const auto started = Clock::now();
    function();
    return std::chrono::duration<double, std::milli>(Clock::now() - started).count();
}

Summary summarize(std::vector<double> values) {
    std::sort(values.begin(), values.end());
    double total = 0.0;
    for (const double value : values) total += value;
    const auto at = [&](double fraction) {
        return values[std::min(values.size() - 1,
            static_cast<size_t>(std::floor(static_cast<double>(values.size()) * fraction)))];
    };
    return {total / static_cast<double>(values.size()), at(0.5), at(0.9), at(0.99), values.back()};
}

void writeSummary(std::ostream& output, const Summary& value) {
    output << "{\"mean\":" << value.mean << ",\"p50\":" << value.p50
           << ",\"p90\":" << value.p90 << ",\"p99\":" << value.p99
           << ",\"max\":" << value.max << '}';
}

long memoryKiB(const char* field) {
    std::ifstream status("/proc/self/status");
    std::string line;
    const std::string prefix = std::string(field) + ':';
    while (std::getline(status, line)) {
        if (line.rfind(prefix, 0) != 0) continue;
        std::istringstream value(line.substr(prefix.size()));
        long kibibytes = -1;
        value >> kibibytes;
        return kibibytes;
    }
    return -1;
}

Geometry loadGeometry(const char* path) {
    std::ifstream input(path, std::ios::binary);
    Geometry geometry;
    input.read(reinterpret_cast<char*>(&geometry.header), sizeof(geometry.header));
    if (!input || geometry.header.magic != kMagic || geometry.header.version != 1 ||
        geometry.header.materialCount != 6 || geometry.header.vertexCount == 0 ||
        geometry.header.triangleCount == 0) {
        throw std::runtime_error("invalid Bistro acoustic geometry");
    }
    std::vector<float> positions(static_cast<size_t>(geometry.header.vertexCount) * 3);
    std::vector<uint32_t> indices(static_cast<size_t>(geometry.header.triangleCount) * 3);
    std::vector<uint8_t> materialIndices(geometry.header.triangleCount);
    input.read(reinterpret_cast<char*>(positions.data()), static_cast<std::streamsize>(positions.size() * sizeof(float)));
    input.read(reinterpret_cast<char*>(indices.data()), static_cast<std::streamsize>(indices.size() * sizeof(uint32_t)));
    input.read(reinterpret_cast<char*>(materialIndices.data()), static_cast<std::streamsize>(materialIndices.size()));
    if (!input) throw std::runtime_error("truncated Bistro acoustic geometry");
    geometry.vertices.resize(geometry.header.vertexCount);
    for (size_t index = 0; index < geometry.vertices.size(); ++index)
        geometry.vertices[index] = {positions[index * 3], positions[index * 3 + 1], positions[index * 3 + 2]};
    geometry.triangles.resize(geometry.header.triangleCount);
    geometry.materialIndices.resize(geometry.header.triangleCount);
    for (size_t index = 0; index < geometry.triangles.size(); ++index) {
        geometry.triangles[index] = {static_cast<IPLint32>(indices[index * 3]),
                                     static_cast<IPLint32>(indices[index * 3 + 1]),
                                     static_cast<IPLint32>(indices[index * 3 + 2])};
        geometry.materialIndices[index] = materialIndices[index];
    }
    return geometry;
}

IPLCoordinateSpace3 coordinates(float x, float y, float z) {
    IPLCoordinateSpace3 value{};
    value.ahead = {0.0f, 0.0f, -1.0f};
    value.up = {0.0f, 1.0f, 0.0f};
    value.right = {1.0f, 0.0f, 0.0f};
    value.origin = {x, y, z};
    return value;
}

void require(IPLerror status, const char* operation) {
    if (status != IPL_STATUS_SUCCESS) throw std::runtime_error(std::string(operation) + " failed: " + std::to_string(status));
}

void runWorker(const char* path, int simulationThreads, const std::string& rayTracer) {
    if (simulationThreads < 1 || simulationThreads > 16) throw std::runtime_error("invalid thread count");
    if (rayTracer != "default" && rayTracer != "embree") throw std::runtime_error("invalid ray tracer");
    const IPLSceneType sceneType = rayTracer == "embree" ? IPL_SCENETYPE_EMBREE : IPL_SCENETYPE_DEFAULT;
    Geometry geometry;
    const double geometryLoadMs = elapsedMs([&] { geometry = loadGeometry(path); });

    IPLContext context = nullptr;
    IPLEmbreeDevice embreeDevice = nullptr;
    IPLScene scene = nullptr;
    IPLStaticMesh mesh = nullptr;
    IPLSimulator simulator = nullptr;
    std::vector<IPLSource> sources(kSources, nullptr);
    const auto cleanup = [&] {
        if (simulator) {
            for (auto& source : sources) {
                if (!source) continue;
                iplSourceRemove(source, simulator);
                iplSourceRelease(&source);
            }
            iplSimulatorRelease(&simulator);
        }
        if (mesh) {
            iplStaticMeshRemove(mesh, scene);
            iplStaticMeshRelease(&mesh);
        }
        if (scene) iplSceneRelease(&scene);
        if (embreeDevice) iplEmbreeDeviceRelease(&embreeDevice);
        if (context) iplContextRelease(&context);
    };
    try {
        IPLContextSettings contextSettings{};
        contextSettings.version = STEAMAUDIO_VERSION;
        contextSettings.simdLevel = IPL_SIMDLEVEL_AVX2;
        require(iplContextCreate(&contextSettings, &context), "context creation");
        double rayTracerDeviceCreateMs = 0.0;
        if (sceneType == IPL_SCENETYPE_EMBREE) {
            IPLEmbreeDeviceSettings embreeSettings{};
            rayTracerDeviceCreateMs = elapsedMs([&] {
                require(iplEmbreeDeviceCreate(context, &embreeSettings, &embreeDevice), "Embree device creation");
            });
        }
        IPLSceneSettings sceneSettings{};
        sceneSettings.type = sceneType;
        sceneSettings.embreeDevice = embreeDevice;
        require(iplSceneCreate(context, &sceneSettings, &scene), "scene creation");
        std::array<IPLMaterial, 6> materials{{
            {{0.20f, 0.30f, 0.40f}, 0.20f, {0.05f, 0.03f, 0.02f}}, // generic
            {{0.10f, 0.05f, 0.02f}, 0.05f, {0.10f, 0.05f, 0.02f}}, // glass
            {{0.40f, 0.60f, 0.70f}, 0.50f, {0.02f, 0.01f, 0.01f}}, // fabric
            {{0.15f, 0.11f, 0.10f}, 0.30f, {0.03f, 0.02f, 0.01f}}, // wood
            {{0.05f, 0.04f, 0.03f}, 0.10f, {0.01f, 0.01f, 0.01f}}, // metal
            {{0.10f, 0.05f, 0.03f}, 0.20f, {0.02f, 0.01f, 0.01f}}, // masonry
        }};
        IPLStaticMeshSettings meshSettings{};
        meshSettings.numVertices = static_cast<IPLint32>(geometry.vertices.size());
        meshSettings.numTriangles = static_cast<IPLint32>(geometry.triangles.size());
        meshSettings.numMaterials = static_cast<IPLint32>(materials.size());
        meshSettings.vertices = geometry.vertices.data();
        meshSettings.triangles = geometry.triangles.data();
        meshSettings.materialIndices = geometry.materialIndices.data();
        meshSettings.materials = materials.data();
        const double staticMeshCreateMs = elapsedMs([&] {
            require(iplStaticMeshCreate(scene, &meshSettings, &mesh), "static mesh creation");
        });
        iplStaticMeshAdd(mesh, scene);
        const double sceneCommitMs = elapsedMs([&] { iplSceneCommit(scene); });
        const long sceneRssKiB = memoryKiB("VmRSS");

        IPLSimulationSettings settings{};
        settings.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
        settings.sceneType = sceneType;
        settings.reflectionType = IPL_REFLECTIONEFFECTTYPE_PARAMETRIC;
        settings.maxNumRays = 1024;
        settings.numDiffuseSamples = 32;
        settings.maxDuration = 0.5f;
        settings.maxOrder = 0;
        settings.maxNumSources = kSources;
        settings.numThreads = simulationThreads;
        settings.rayBatchSize = 1;
        settings.samplingRate = 48000;
        settings.frameSize = 128;
        const double simulatorCreateMs = elapsedMs([&] {
            require(iplSimulatorCreate(context, &settings, &simulator), "simulator creation");
        });
        iplSimulatorSetScene(simulator, scene);
        iplSimulatorCommit(simulator);
        IPLSourceSettings sourceSettings{};
        sourceSettings.flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
        std::vector<IPLSimulationInputs> inputs(kSources);
        std::vector<IPLSimulationOutputs> outputs(kSources);
        for (int index = 0; index < kSources; ++index) {
            require(iplSourceCreate(simulator, &sourceSettings, &sources[index]), "source creation");
            iplSourceAdd(sources[index], simulator);
            inputs[index].flags = IPL_SIMULATIONFLAGS_REFLECTIONS;
            inputs[index].source = coordinates(geometry.header.source[0] + static_cast<float>(index % 8) * 0.08f,
                                               geometry.header.source[1],
                                               geometry.header.source[2] + static_cast<float>(index / 8) * 0.08f);
            inputs[index].reverbScale[0] = inputs[index].reverbScale[1] = inputs[index].reverbScale[2] = 1.0f;
            inputs[index].baked = IPL_FALSE;
            iplSourceSetInputs(sources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &inputs[index]);
        }
        iplSimulatorCommit(simulator);
        IPLSimulationSharedInputs shared{};
        shared.listener = coordinates(geometry.header.listener[0], geometry.header.listener[1], geometry.header.listener[2]);
        shared.numBounces = 2;
        shared.duration = 0.5f;
        shared.order = 0;
        shared.irradianceMinDistance = 1.0f;

        std::ostringstream scenarios;
        scenarios << '[';
        for (int scenarioIndex = 0; scenarioIndex < 2; ++scenarioIndex) {
            const int rays = scenarioIndex == 0 ? 512 : 1024;
            shared.numRays = rays;
            iplSimulatorSetSharedInputs(simulator, IPL_SIMULATIONFLAGS_REFLECTIONS, &shared);
            for (int warmup = 0; warmup < 3; ++warmup) iplSimulatorRunReflections(simulator);
            std::vector<double> samples;
            samples.reserve(kSamples);
            float reverbMinimum = INFINITY;
            float reverbMaximum = -INFINITY;
            bool allIrValid = true;
            for (int sample = 0; sample < kSamples; ++sample) {
                const float phase = static_cast<float>(sample) * 0.173f;
                shared.listener = coordinates(geometry.header.listener[0] + std::sin(phase) * 0.05f,
                                              geometry.header.listener[1],
                                              geometry.header.listener[2] + std::cos(phase) * 0.05f);
                iplSimulatorSetSharedInputs(simulator, IPL_SIMULATIONFLAGS_REFLECTIONS, &shared);
                samples.push_back(elapsedMs([&] { iplSimulatorRunReflections(simulator); }));
                for (int index = 0; index < kSources; ++index)
                    iplSourceGetOutputs(sources[index], IPL_SIMULATIONFLAGS_REFLECTIONS, &outputs[index]);
                reverbMinimum = std::min(reverbMinimum, outputs[0].reflections.reverbTimes[0]);
                reverbMaximum = std::max(reverbMaximum, outputs[0].reflections.reverbTimes[0]);
                allIrValid = allIrValid && outputs[0].reflections.ir;
            }
            if (scenarioIndex) scenarios << ',';
            scenarios << "{\"rays\":" << rays << ",\"bounces\":2,\"samples\":" << kSamples
                      << ",\"reflectionSimulation\":";
            writeSummary(scenarios, summarize(std::move(samples)));
            scenarios << ",\"reverbLowRange\":[" << reverbMinimum << ',' << reverbMaximum << ']'
                      << ",\"irValid\":" << (allIrValid ? "true" : "false") << '}';
        }
        scenarios << ']';
        const auto asset = std::filesystem::path(path).filename().string();
        std::cout << std::setprecision(12)
                  << "BISTRO_RESULTS {\"worker\":\"std::thread\",\"steamAudioThreads\":" << simulationThreads
                  << ",\"rayTracer\":\"" << rayTracer << "\""
                  << ",\"asset\":\"Amazon Lumberyard " << asset << " v5.2\",\"vertices\":" << geometry.header.vertexCount
                  << ",\"triangles\":" << geometry.header.triangleCount
                  << ",\"bounds\":{\"min\":[" << geometry.header.minimum[0] << ',' << geometry.header.minimum[1] << ',' << geometry.header.minimum[2]
                  << "],\"max\":[" << geometry.header.maximum[0] << ',' << geometry.header.maximum[1] << ',' << geometry.header.maximum[2] << "]}"
                  << ",\"geometryLoadMs\":" << geometryLoadMs
                  << ",\"rayTracerDeviceCreateMs\":" << rayTracerDeviceCreateMs
                  << ",\"staticMeshCreateMs\":" << staticMeshCreateMs
                  << ",\"sceneCommitMs\":" << sceneCommitMs
                  << ",\"simulatorCreateMs\":" << simulatorCreateMs
                  << ",\"sceneRssKiB\":" << sceneRssKiB
                  << ",\"peakRssKiB\":" << memoryKiB("VmHWM")
                  << ",\"scenarios\":" << scenarios.str() << "}\n";
        cleanup();
    } catch (...) {
        cleanup();
        throw;
    }
}
}

int main(int argc, char** argv) {
    if (argc != 4) {
        std::cerr << "usage: native-bistro-geometry-benchmark <bistro-acoustic.bin> <simulation-threads> <default|embree>\n";
        return 2;
    }
    const int threads = std::stoi(argv[2]);
    const std::string rayTracer = argv[3];
    std::exception_ptr failure;
    std::thread worker([&] {
        try {
            runWorker(argv[1], threads, rayTracer);
        } catch (...) {
            failure = std::current_exception();
        }
    });
    worker.join();
    if (!failure) return 0;
    try {
        std::rethrow_exception(failure);
    } catch (const std::exception& error) {
        std::cerr << "native Bistro worker failed: " << error.what() << '\n';
    }
    return 1;
}
