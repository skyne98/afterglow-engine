#include <phonon.h>
#include <emscripten/emscripten.h>
#include <vector>

namespace {
IPLContext gContext = nullptr;
IPLScene gScene = nullptr;
IPLStaticMesh gMesh = nullptr;
IPLSimulator gSimulator = nullptr;
std::vector<IPLSource> gSources;
std::vector<IPLSimulationInputs> gSourceInputs;
IPLSimulationSharedInputs gSharedInputs{};
IPLSimulationOutputs gOutputs{};

IPLCoordinateSpace3 coordinates(float x, float y, float z) {
    IPLCoordinateSpace3 value{};
    value.ahead = {0.0f, 0.0f, -1.0f};
    value.up = {0.0f, 1.0f, 0.0f};
    value.right = {1.0f, 0.0f, 0.0f};
    value.origin = {x, y, z};
    return value;
}

void releaseAll() {
    for (auto& source : gSources) {
        if (source) {
            iplSourceRemove(source, gSimulator);
            iplSourceRelease(&source);
        }
    }
    gSources.clear();
    gSourceInputs.clear();
    if (gSimulator) iplSimulatorRelease(&gSimulator);
    if (gMesh) {
        iplStaticMeshRemove(gMesh, gScene);
        iplStaticMeshRelease(&gMesh);
    }
    if (gScene) iplSceneRelease(&gScene);
    if (gContext) iplContextRelease(&gContext);
}
}

extern "C" {

EMSCRIPTEN_KEEPALIVE int sa_init(int triangleCount, int sourceCount) {
    releaseAll();
    if (triangleCount < 2 || sourceCount < 1) return 100;

    IPLContextSettings contextSettings{};
    contextSettings.version = STEAMAUDIO_VERSION;
    contextSettings.simdLevel = IPL_SIMDLEVEL_NEON;
    if (iplContextCreate(&contextSettings, &gContext) != IPL_STATUS_SUCCESS) return 1;

    IPLSceneSettings sceneSettings{};
    sceneSettings.type = IPL_SCENETYPE_DEFAULT;
    if (iplSceneCreate(gContext, &sceneSettings, &gScene) != IPL_STATUS_SUCCESS) return 2;

    // Triangle 0/1 form the actual 4 m x 4 m occluder in the yz plane.
    // Remaining triangles are deterministic distant geometry used to measure
    // traversal scaling without intersecting the source-listener segment.
    std::vector<IPLVector3> vertices(static_cast<size_t>(triangleCount) * 3);
    std::vector<IPLTriangle> triangles(static_cast<size_t>(triangleCount));
    vertices[0] = {0.0f, -2.0f, -2.0f};
    vertices[1] = {0.0f, 2.0f, -2.0f};
    vertices[2] = {0.0f, 2.0f, 2.0f};
    vertices[3] = {0.0f, -2.0f, -2.0f};
    vertices[4] = {0.0f, 2.0f, 2.0f};
    vertices[5] = {0.0f, -2.0f, 2.0f};
    triangles[0] = {0, 1, 2};
    triangles[1] = {3, 4, 5};
    for (int index = 2; index < triangleCount; ++index) {
        const int vertex = index * 3;
        const float x = 20.0f + static_cast<float>(index % 317) * 0.07f;
        const float y = 20.0f + static_cast<float>((index / 317) % 317) * 0.07f;
        const float z = static_cast<float>(index % 97) * 0.03f;
        vertices[vertex] = {x, y, z};
        vertices[vertex + 1] = {x + 0.02f, y, z};
        vertices[vertex + 2] = {x, y + 0.02f, z};
        triangles[index] = {vertex, vertex + 1, vertex + 2};
    }
    IPLMaterial materials[1] = {{{0.20f, 0.30f, 0.40f}, 0.10f, {0.05f, 0.03f, 0.01f}}};
    std::vector<IPLint32> materialIndices(static_cast<size_t>(triangleCount), 0);
    IPLStaticMeshSettings meshSettings{};
    meshSettings.numVertices = triangleCount * 3;
    meshSettings.numTriangles = triangleCount;
    meshSettings.numMaterials = 1;
    meshSettings.vertices = vertices.data();
    meshSettings.triangles = triangles.data();
    meshSettings.materialIndices = materialIndices.data();
    meshSettings.materials = materials;
    if (iplStaticMeshCreate(gScene, &meshSettings, &gMesh) != IPL_STATUS_SUCCESS) return 3;
    iplStaticMeshAdd(gMesh, gScene);
    iplSceneCommit(gScene);

    IPLSimulationSettings simulationSettings{};
    simulationSettings.flags = IPL_SIMULATIONFLAGS_DIRECT;
    simulationSettings.sceneType = IPL_SCENETYPE_DEFAULT;
    simulationSettings.maxNumSources = sourceCount;
    if (iplSimulatorCreate(gContext, &simulationSettings, &gSimulator) != IPL_STATUS_SUCCESS) return 4;
    iplSimulatorSetScene(gSimulator, gScene);
    iplSimulatorCommit(gSimulator);

    gSources.resize(static_cast<size_t>(sourceCount), nullptr);
    gSourceInputs.resize(static_cast<size_t>(sourceCount));
    IPLSourceSettings sourceSettings{};
    sourceSettings.flags = IPL_SIMULATIONFLAGS_DIRECT;
    for (int index = 0; index < sourceCount; ++index) {
        if (iplSourceCreate(gSimulator, &sourceSettings, &gSources[index]) != IPL_STATUS_SUCCESS) return 5;
        iplSourceAdd(gSources[index], gSimulator);
        auto& input = gSourceInputs[index];
        input.flags = IPL_SIMULATIONFLAGS_DIRECT;
        input.directFlags = static_cast<IPLDirectSimulationFlags>(
            IPL_DIRECTSIMULATIONFLAGS_OCCLUSION | IPL_DIRECTSIMULATIONFLAGS_TRANSMISSION);
        input.occlusionType = IPL_OCCLUSIONTYPE_RAYCAST;
        input.numTransmissionRays = 1;
        input.source = coordinates(-1.0f, static_cast<float>(index % 16) * 0.02f, 0.0f);
        iplSourceSetInputs(gSources[index], IPL_SIMULATIONFLAGS_DIRECT, &input);
    }
    iplSimulatorCommit(gSimulator);

    gSharedInputs.listener = coordinates(1.0f, 0.0f, 0.0f);
    iplSimulatorSetSharedInputs(gSimulator, IPL_SIMULATIONFLAGS_DIRECT, &gSharedInputs);
    iplSimulatorRunDirect(gSimulator);
    iplSourceGetOutputs(gSources[0], IPL_SIMULATIONFLAGS_DIRECT, &gOutputs);
    return 0;
}

EMSCRIPTEN_KEEPALIVE void sa_set_occluded(int occluded) {
    for (size_t index = 0; index < gSources.size(); ++index) {
        auto& input = gSourceInputs[index];
        input.source = coordinates(occluded ? -1.0f : 0.5f,
                                   static_cast<float>(index % 16) * 0.02f, 0.0f);
        iplSourceSetInputs(gSources[index], IPL_SIMULATIONFLAGS_DIRECT, &input);
    }
}

EMSCRIPTEN_KEEPALIVE void sa_run_direct() {
    iplSimulatorRunDirect(gSimulator);
    iplSourceGetOutputs(gSources[0], IPL_SIMULATIONFLAGS_DIRECT, &gOutputs);
}

EMSCRIPTEN_KEEPALIVE void sa_run_direct_batch(int iterations) {
    for (int index = 0; index < iterations; ++index) iplSimulatorRunDirect(gSimulator);
    iplSourceGetOutputs(gSources[0], IPL_SIMULATIONFLAGS_DIRECT, &gOutputs);
}

EMSCRIPTEN_KEEPALIVE float sa_get_occlusion() { return gOutputs.direct.occlusion; }
EMSCRIPTEN_KEEPALIVE float sa_get_transmission_low() { return gOutputs.direct.transmission[0]; }
EMSCRIPTEN_KEEPALIVE float sa_get_transmission_mid() { return gOutputs.direct.transmission[1]; }
EMSCRIPTEN_KEEPALIVE float sa_get_transmission_high() { return gOutputs.direct.transmission[2]; }
EMSCRIPTEN_KEEPALIVE void sa_shutdown() { releaseAll(); }

}
