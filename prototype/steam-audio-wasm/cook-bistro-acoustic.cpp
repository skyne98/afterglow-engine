#include <assimp/Importer.hpp>
#include <assimp/material.h>
#include <assimp/postprocess.h>
#include <assimp/scene.h>
#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <string>
#include <vector>

namespace {
constexpr std::array<char, 8> kMagic{'A', 'G', 'B', 'I', 'S', 'T', '1', '\0'};

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

std::string lower(std::string value) {
    std::transform(value.begin(), value.end(), value.begin(),
                   [](unsigned char character) { return static_cast<char>(std::tolower(character)); });
    return value;
}

uint8_t acousticCategory(const aiMaterial* material) {
    aiString name;
    material->Get(AI_MATKEY_NAME, name);
    const auto value = lower(name.C_Str());
    if (value.find("glass") != std::string::npos || value.find("window") != std::string::npos) return 1;
    if (value.find("fabric") != std::string::npos || value.find("cloth") != std::string::npos ||
        value.find("curtain") != std::string::npos || value.find("carpet") != std::string::npos) return 2;
    if (value.find("wood") != std::string::npos) return 3;
    if (value.find("metal") != std::string::npos || value.find("steel") != std::string::npos ||
        value.find("brass") != std::string::npos) return 4;
    if (value.find("concrete") != std::string::npos || value.find("brick") != std::string::npos ||
        value.find("plaster") != std::string::npos || value.find("stone") != std::string::npos) return 5;
    return 0;
}

template <typename Value>
void writeValues(std::ofstream& output, const std::vector<Value>& values) {
    output.write(reinterpret_cast<const char*>(values.data()),
                 static_cast<std::streamsize>(values.size() * sizeof(Value)));
}
}

int main(int argc, char** argv) {
    if (argc != 3) {
        std::cerr << "usage: cook-bistro-acoustic <BistroInterior.fbx> <output.bin>\n";
        return 2;
    }
    Assimp::Importer importer;
    const auto* scene = importer.ReadFile(argv[1], aiProcess_Triangulate |
                                                   aiProcess_JoinIdenticalVertices |
                                                   aiProcess_PreTransformVertices |
                                                   aiProcess_SortByPType |
                                                   aiProcess_ValidateDataStructure);
    if (!scene) {
        std::cerr << "Bistro import failed: " << importer.GetErrorString() << '\n';
        return 1;
    }
    std::vector<float> positions;
    std::vector<uint32_t> indices;
    std::vector<uint8_t> materialIndices;
    uint64_t totalVertices = 0;
    uint64_t totalTriangles = 0;
    for (uint32_t index = 0; index < scene->mNumMeshes; ++index) {
        totalVertices += scene->mMeshes[index]->mNumVertices;
        totalTriangles += scene->mMeshes[index]->mNumFaces;
    }
    if (totalVertices > std::numeric_limits<uint32_t>::max() ||
        totalTriangles > std::numeric_limits<uint32_t>::max()) {
        std::cerr << "Bistro geometry exceeds compact format limits\n";
        return 1;
    }
    positions.reserve(static_cast<size_t>(totalVertices) * 3);
    indices.reserve(static_cast<size_t>(totalTriangles) * 3);
    materialIndices.reserve(static_cast<size_t>(totalTriangles));
    float minimum[3]{INFINITY, INFINITY, INFINITY};
    float maximum[3]{-INFINITY, -INFINITY, -INFINITY};
    for (uint32_t meshIndex = 0; meshIndex < scene->mNumMeshes; ++meshIndex) {
        const auto* mesh = scene->mMeshes[meshIndex];
        const auto base = static_cast<uint32_t>(positions.size() / 3);
        for (uint32_t vertexIndex = 0; vertexIndex < mesh->mNumVertices; ++vertexIndex) {
            const auto& vertex = mesh->mVertices[vertexIndex];
            const float values[3]{vertex.x, vertex.y, vertex.z};
            for (int axis = 0; axis < 3; ++axis) {
                positions.push_back(values[axis]);
                minimum[axis] = std::min(minimum[axis], values[axis]);
                maximum[axis] = std::max(maximum[axis], values[axis]);
            }
        }
        const uint8_t material = acousticCategory(scene->mMaterials[mesh->mMaterialIndex]);
        for (uint32_t faceIndex = 0; faceIndex < mesh->mNumFaces; ++faceIndex) {
            const auto& face = mesh->mFaces[faceIndex];
            if (face.mNumIndices != 3) continue;
            indices.push_back(base + face.mIndices[0]);
            indices.push_back(base + face.mIndices[1]);
            indices.push_back(base + face.mIndices[2]);
            materialIndices.push_back(material);
        }
    }
    Header header{};
    header.magic = kMagic;
    header.version = 1;
    header.vertexCount = static_cast<uint32_t>(positions.size() / 3);
    header.triangleCount = static_cast<uint32_t>(indices.size() / 3);
    header.materialCount = 6;
    std::copy_n(minimum, 3, header.minimum);
    std::copy_n(maximum, 3, header.maximum);
    if (scene->mNumCameras != 1) {
        std::cerr << "expected exactly one authored Bistro camera\n";
        return 1;
    }
    // aiProcess_PreTransformVertices also places the authored camera in flattened scene space.
    const auto& camera = scene->mCameras[0]->mPosition;
    const std::string inputPath = argv[1];
    if (inputPath.ends_with("BistroInterior.fbx")) {
        // Preserve the originally published interior benchmark placement.
        header.listener[0] = 0.0521628f;
        header.listener[1] = 1.44433f;
        header.listener[2] = -1.42984f;
        header.source[0] = 2.0f;
        header.source[1] = 1.4f;
        header.source[2] = -1.4f;
    } else {
        header.listener[0] = camera.x;
        header.listener[1] = camera.y;
        header.listener[2] = camera.z;
        header.source[0] = camera.x + 1.95f;
        header.source[1] = camera.y;
        header.source[2] = camera.z;
    }

    std::ofstream output(argv[2], std::ios::binary | std::ios::trunc);
    output.write(reinterpret_cast<const char*>(&header), sizeof(header));
    writeValues(output, positions);
    writeValues(output, indices);
    writeValues(output, materialIndices);
    if (!output) {
        std::cerr << "failed to write acoustic geometry\n";
        return 1;
    }
    std::cout << "cooked " << header.vertexCount << " vertices, " << header.triangleCount
              << " triangles from official Bistro scene\n";
}
