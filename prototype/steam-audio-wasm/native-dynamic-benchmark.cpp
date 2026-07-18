#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <exception>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

extern "C" {
int dyn_set_simulation_threads(int threads);
int dyn_init(int triangleCount, int sourceCount, int maxRays, int maxBounces,
             int reflectionType, int maxDurationMs, int maxOrder);
int dyn_update(float phase);
int dyn_run_reflections(int rays, int bounces, int durationMs, int order);
int dyn_run_audio(int iterations);
int dyn_run_binaural(int iterations);
float dyn_get_reverb_low();
float dyn_get_reverb_mid();
float dyn_get_reverb_high();
int dyn_get_ir_valid();
float dyn_get_output_energy();
void dyn_shutdown();
}

namespace {
using Clock = std::chrono::steady_clock;

struct Scenario {
    const char* name;
    int triangles;
    int sources;
    int rays;
    int bounces;
    int durationMs;
    int order;
    int reflectionType;
    int samples;
};

struct Summary {
    double mean;
    double p50;
    double p90;
    double p99;
    double max;
};

constexpr Scenario kScenarios[] = {
    {"p16-512x1", 10'000, 16, 512, 1, 250, 0, 1, 30},
    {"p16-1024x2", 10'000, 16, 1024, 2, 500, 0, 1, 30},
    {"p16-2048x2", 10'000, 16, 2048, 2, 500, 0, 1, 30},
    {"p32-256x1", 10'000, 32, 256, 1, 250, 0, 1, 30},
    {"p32-512x1", 10'000, 32, 512, 1, 250, 0, 1, 30},
    {"p32-512x2", 10'000, 32, 512, 2, 500, 0, 1, 30},
    {"p32-1024x1", 10'000, 32, 1024, 1, 250, 0, 1, 30},
    {"p32-1024x2", 10'000, 32, 1024, 2, 500, 0, 1, 30},
    {"p32-2048x2", 10'000, 32, 2048, 2, 500, 0, 1, 30},
    {"p32-2048x4", 10'000, 32, 2048, 4, 1000, 0, 1, 30},
    {"p64-256x1", 10'000, 64, 256, 1, 250, 0, 1, 30},
    {"p64-512x1", 10'000, 64, 512, 1, 250, 0, 1, 30},
    {"p64-256x2", 10'000, 64, 256, 2, 500, 0, 1, 30},
    {"p64-384x2", 10'000, 64, 384, 2, 500, 0, 1, 30},
    {"p64-512x2", 10'000, 64, 512, 2, 500, 0, 1, 30},
    {"p64-1024x1", 10'000, 64, 1024, 1, 250, 0, 1, 30},
    {"p64-1024x2", 10'000, 64, 1024, 2, 500, 0, 1, 30},
    {"p64-2048x2", 10'000, 64, 2048, 2, 500, 0, 1, 30},
    {"p96-256x2", 10'000, 96, 256, 2, 500, 0, 1, 30},
    {"p96-512x2", 10'000, 96, 512, 2, 500, 0, 1, 30},
    {"p128-128x2", 10'000, 128, 128, 2, 500, 0, 1, 30},
    {"p128-256x2", 10'000, 128, 256, 2, 500, 0, 1, 30},
    {"p128-512x2", 10'000, 128, 512, 2, 500, 0, 1, 30},
    {"c32-256x1-o0", 10'000, 32, 256, 1, 250, 0, 0, 20},
    {"c32-512x2-o0", 10'000, 32, 512, 2, 500, 0, 0, 20},
    {"c32-512x2-o1", 10'000, 32, 512, 2, 500, 1, 0, 20},
    {"c64-256x1-o0", 10'000, 64, 256, 1, 250, 0, 0, 20},
    {"c64-512x2-o0", 10'000, 64, 512, 2, 500, 0, 0, 20},
    {"c64-512x2-o1", 10'000, 64, 512, 2, 500, 1, 0, 20},
};

template <typename Function>
double elapsedMs(Function&& function) {
    const auto started = Clock::now();
    const int status = function();
    const auto finished = Clock::now();
    if (status != 0) throw std::runtime_error("Steam Audio status " + std::to_string(status));
    return std::chrono::duration<double, std::milli>(finished - started).count();
}

Summary summarize(std::vector<double> values) {
    std::sort(values.begin(), values.end());
    double total = 0.0;
    for (const double value : values) total += value;
    const auto at = [&](double fraction) {
        const auto index = std::min(values.size() - 1,
            static_cast<size_t>(std::floor(static_cast<double>(values.size()) * fraction)));
        return values[index];
    };
    return {total / static_cast<double>(values.size()), at(0.5), at(0.9), at(0.99), values.back()};
}

void writeSummary(std::ostream& output, const Summary& summary) {
    output << "{\"mean\":" << summary.mean
           << ",\"p50\":" << summary.p50
           << ",\"p90\":" << summary.p90
           << ",\"p99\":" << summary.p99
           << ",\"max\":" << summary.max << '}';
}

long peakRssKiB() {
    std::ifstream status("/proc/self/status");
    std::string line;
    while (std::getline(status, line)) {
        if (line.rfind("VmHWM:", 0) != 0) continue;
        std::istringstream value(line.substr(6));
        long kibibytes = 0;
        value >> kibibytes;
        return kibibytes;
    }
    return -1;
}

void runWorker(int steamAudioThreads) {
    if (dyn_set_simulation_threads(steamAudioThreads) != 0)
        throw std::runtime_error("invalid Steam Audio thread count");
    std::ostringstream results;
    results << std::setprecision(12) << '[';
    bool first = true;
    for (const auto& scenario : kScenarios) {
        const double initializationMs = elapsedMs([&] {
            return dyn_init(scenario.triangles, scenario.sources, scenario.rays,
                            scenario.bounces, scenario.reflectionType,
                            scenario.durationMs, scenario.order);
        });
        for (int index = 0; index < 3; ++index) {
            elapsedMs([&] { return dyn_update(static_cast<float>(index) * 0.31f); });
            elapsedMs([&] { return dyn_run_reflections(scenario.rays, scenario.bounces,
                                                       scenario.durationMs, scenario.order); });
        }
        std::vector<double> updates;
        std::vector<double> simulations;
        updates.reserve(static_cast<size_t>(scenario.samples));
        simulations.reserve(static_cast<size_t>(scenario.samples));
        float reverbLowMin = INFINITY;
        float reverbLowMax = -INFINITY;
        for (int index = 0; index < scenario.samples; ++index) {
            const float phase = static_cast<float>(index) * 0.173f;
            updates.push_back(elapsedMs([&] { return dyn_update(phase); }));
            simulations.push_back(elapsedMs([&] {
                return dyn_run_reflections(scenario.rays, scenario.bounces,
                                           scenario.durationMs, scenario.order);
            }));
            reverbLowMin = std::min(reverbLowMin, dyn_get_reverb_low());
            reverbLowMax = std::max(reverbLowMax, dyn_get_reverb_low());
        }
        const int reflectionIterations = scenario.reflectionType == 0 ? 100 : 1000;
        const double reflectionQuantumMeanMs = elapsedMs([&] {
            return dyn_run_audio(reflectionIterations);
        }) / static_cast<double>(reflectionIterations);
        const int binauralIterations = 100;
        const double binauralQuantumMeanMs = elapsedMs([&] {
            return dyn_run_binaural(binauralIterations);
        }) / static_cast<double>(binauralIterations);
        if (!first) results << ',';
        first = false;
        results << "{\"name\":\"" << scenario.name
                << "\",\"triangles\":" << scenario.triangles
                << ",\"sources\":" << scenario.sources
                << ",\"rays\":" << scenario.rays
                << ",\"bounces\":" << scenario.bounces
                << ",\"durationMs\":" << scenario.durationMs
                << ",\"order\":" << scenario.order
                << ",\"reflectionType\":" << scenario.reflectionType
                << ",\"samples\":" << scenario.samples
                << ",\"initializationMs\":" << initializationMs
                << ",\"sceneUpdate\":";
        writeSummary(results, summarize(std::move(updates)));
        results << ",\"reflectionSimulation\":";
        writeSummary(results, summarize(std::move(simulations)));
        results << ",\"reflectionQuantumMeanMs\":" << reflectionQuantumMeanMs
                << ",\"binauralQuantumMeanMs\":" << binauralQuantumMeanMs
                << ",\"combinedAudioQuantumMeanMs\":" << reflectionQuantumMeanMs + binauralQuantumMeanMs
                << ",\"reverb\":[" << dyn_get_reverb_low() << ',' << dyn_get_reverb_mid() << ',' << dyn_get_reverb_high() << ']'
                << ",\"reverbLowRange\":[" << reverbLowMin << ',' << reverbLowMax << ']'
                << ",\"irValid\":" << (dyn_get_ir_valid() ? "true" : "false")
                << ",\"outputEnergy\":" << dyn_get_output_energy() << '}';
    }
    dyn_shutdown();
    results << ']';
    std::cout << "NATIVE_RESULTS {\"worker\":\"std::thread\",\"steamAudioThreads\":" << steamAudioThreads
              << ",\"peakRssKiB\":"
              << peakRssKiB() << ",\"results\":" << results.str() << "}\n";
}
}

int main(int argc, char** argv) {
    const int steamAudioThreads = argc > 1 ? std::stoi(argv[1]) : 1;
    std::exception_ptr failure;
    std::thread worker([&] {
        try {
            runWorker(steamAudioThreads);
        } catch (...) {
            failure = std::current_exception();
            dyn_shutdown();
        }
    });
    worker.join();
    if (failure) {
        try {
            std::rethrow_exception(failure);
        } catch (const std::exception& error) {
            std::cerr << "native Steam Audio worker failed: " << error.what() << '\n';
        }
        return 1;
    }
    return 0;
}
