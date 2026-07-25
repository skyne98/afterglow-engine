#include <emscripten/emscripten.h>
#include <emscripten/webaudio.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <thread>

extern "C" {
int afterglow_steam_audio_update_motion(float phase);
int afterglow_steam_audio_run_direct_simulation();
int afterglow_steam_audio_run_reflection_simulation();
int afterglow_steam_audio_render_quantum();
const float* afterglow_steam_audio_pcm_ptr();
}

namespace {
constexpr int kFrames = 128;
constexpr int kChannels = 2;
constexpr char kProcessorName[] = "afterglow-steam-audio-wasm-gate";

alignas(16) std::uint8_t gAudioStack[64 * 1024];
EMSCRIPTEN_WEBAUDIO_T gAudioContext = 0;
EMSCRIPTEN_AUDIO_WORKLET_NODE_T gAudioNode = 0;
std::atomic<std::uint32_t> gStatus{0};
std::atomic<std::uint32_t> gCallbacks{0};
std::atomic<std::uint32_t> gErrors{0};
std::atomic<std::uint32_t> gMaxMicros{0};
std::atomic<std::uint32_t> gOverBudget{0};
std::atomic<std::uint32_t> gMaxGapMicros{0};
double gLastCallbackMs = 0.0;
std::atomic<std::uint32_t> gEnergyBits{0};
std::atomic<std::uint32_t> gPeakBits{0};
std::thread gSimulationThread;
std::atomic<bool> gSimulationStarted{false};
std::atomic<bool> gSimulationStop{false};
std::atomic<bool> gSimulationRunning{false};
std::atomic<std::uint32_t> gSimulationUpdates{0};
std::atomic<std::uint32_t> gReflectionSimulationUpdates{0};
std::atomic<std::uint32_t> gSimulationErrors{0};
std::atomic<std::uint32_t> gSimulationMaxMicros{0};

void storeFloat(std::atomic<std::uint32_t>& destination, float value) {
    std::uint32_t bits = 0;
    static_assert(sizeof(bits) == sizeof(value));
    std::memcpy(&bits, &value, sizeof(bits));
    destination.store(bits, std::memory_order_relaxed);
}

float loadFloat(const std::atomic<std::uint32_t>& source) {
    const std::uint32_t bits = source.load(std::memory_order_relaxed);
    float value = 0.0f;
    std::memcpy(&value, &bits, sizeof(value));
    return value;
}

void runSimulationWorker() {
    gSimulationRunning.store(true, std::memory_order_release);
    float phase = 0.0f;
    auto nextUpdate = std::chrono::steady_clock::now();
    std::uint32_t updateIndex = 0;
    while (!gSimulationStop.load(std::memory_order_acquire)) {
        const double started = emscripten_get_now();
        phase += 0.025f;
        int simulationStatus = afterglow_steam_audio_update_motion(phase);
        if (simulationStatus == 0)
            simulationStatus = afterglow_steam_audio_run_direct_simulation();
        if (simulationStatus == 0 && updateIndex % 5 == 0) {
            simulationStatus = afterglow_steam_audio_run_reflection_simulation();
            if (simulationStatus == 0)
                gReflectionSimulationUpdates.fetch_add(1, std::memory_order_relaxed);
        }
        ++updateIndex;
        const auto micros = static_cast<std::uint32_t>(
            std::max(0.0, (emscripten_get_now() - started) * 1000.0));
        auto prior = gSimulationMaxMicros.load(std::memory_order_relaxed);
        while (micros > prior && !gSimulationMaxMicros.compare_exchange_weak(
                   prior, micros, std::memory_order_relaxed,
                   std::memory_order_relaxed)) {}
        if (simulationStatus != 0) {
            gSimulationErrors.fetch_add(1, std::memory_order_relaxed);
            break;
        }
        gSimulationUpdates.fetch_add(1, std::memory_order_relaxed);
        nextUpdate += std::chrono::milliseconds(200);
        const auto now = std::chrono::steady_clock::now();
        if (nextUpdate < now) nextUpdate = now;
        std::this_thread::sleep_until(nextUpdate);
    }
    gSimulationRunning.store(false, std::memory_order_release);
}

bool processAudio(int, const AudioSampleFrame*, int numOutputs,
                  AudioSampleFrame* outputs, int, const AudioParamFrame*, void*) {
    const double started = emscripten_get_now();
    if (gCallbacks.load(std::memory_order_relaxed) == 256) {
        // Exclude AudioContext/device startup transients from steady-state
        // deadline telemetry while retaining the total callback count.
        gMaxMicros.store(0, std::memory_order_relaxed);
        gOverBudget.store(0, std::memory_order_relaxed);
        gMaxGapMicros.store(0, std::memory_order_relaxed);
        gLastCallbackMs = started;
    }
    if (gLastCallbackMs != 0.0) {
        const auto gapMicros = static_cast<std::uint32_t>(
            std::max(0.0, (started - gLastCallbackMs) * 1000.0));
        auto priorGap = gMaxGapMicros.load(std::memory_order_relaxed);
        while (gapMicros > priorGap && !gMaxGapMicros.compare_exchange_weak(
                   priorGap, gapMicros, std::memory_order_relaxed,
                   std::memory_order_relaxed)) {}
    }
    gLastCallbackMs = started;
    if (numOutputs != 1 || outputs == nullptr || outputs[0].numberOfChannels != kChannels ||
        outputs[0].samplesPerChannel != kFrames || outputs[0].data == nullptr) {
        gErrors.fetch_add(1, std::memory_order_relaxed);
        if (numOutputs > 0 && outputs != nullptr && outputs[0].data != nullptr) {
            const int count = outputs[0].numberOfChannels * outputs[0].samplesPerChannel;
            std::fill_n(outputs[0].data, count, 0.0f);
        }
        return true;
    }
    if (afterglow_steam_audio_render_quantum() != 0) {
        std::fill_n(outputs[0].data, kFrames * kChannels, 0.0f);
        gErrors.fetch_add(1, std::memory_order_relaxed);
        return true;
    }
    const float* input = afterglow_steam_audio_pcm_ptr();
    float energy = 0.0f;
    float peak = 0.0f;
    for (int frame = 0; frame < kFrames; ++frame) {
        const float left = input[2 * frame];
        const float right = input[2 * frame + 1];
        outputs[0].data[frame] = left;
        outputs[0].data[kFrames + frame] = right;
        energy += std::fabs(left) + std::fabs(right);
        peak = std::max(peak, std::max(std::fabs(left), std::fabs(right)));
    }
    storeFloat(gEnergyBits, energy);
    storeFloat(gPeakBits, peak);
    gCallbacks.fetch_add(1, std::memory_order_relaxed);
    const auto micros = static_cast<std::uint32_t>(
        std::max(0.0, (emscripten_get_now() - started) * 1000.0));
    auto prior = gMaxMicros.load(std::memory_order_relaxed);
    while (micros > prior && !gMaxMicros.compare_exchange_weak(
               prior, micros, std::memory_order_relaxed, std::memory_order_relaxed)) {}
    if (micros > 2'667)
        gOverBudget.fetch_add(1, std::memory_order_relaxed);
    return true;
}

void processorCreated(EMSCRIPTEN_WEBAUDIO_T context, bool success, void*) {
    if (!success) {
        gStatus.store(0x80000002u, std::memory_order_release);
        return;
    }
    int channels[1] = {kChannels};
    EmscriptenAudioWorkletNodeCreateOptions options{};
    options.numberOfInputs = 0;
    options.numberOfOutputs = 1;
    options.outputChannelCounts = channels;
    options.channelCount = kChannels;
    options.channelCountMode = WEBAUDIO_CHANNEL_COUNT_MODE_EXPLICIT;
    options.channelInterpretation = WEBAUDIO_CHANNEL_INTERPRETATION_SPEAKERS;
    gAudioNode = emscripten_create_wasm_audio_worklet_node(
        context, kProcessorName, &options, processAudio, nullptr);
    if (gAudioNode == 0) {
        gStatus.store(0x80000003u, std::memory_order_release);
        return;
    }
    emscripten_audio_node_connect(gAudioNode, context, 0, 0);
    gStatus.store(3, std::memory_order_release);
}

void audioThreadStarted(EMSCRIPTEN_WEBAUDIO_T context, bool success, void*) {
    if (!success) {
        gStatus.store(0x80000001u, std::memory_order_release);
        return;
    }
    WebAudioWorkletProcessorCreateOptions options{};
    options.name = kProcessorName;
    emscripten_create_wasm_audio_worklet_processor_async(
        context, &options, processorCreated, nullptr);
    gStatus.store(2, std::memory_order_release);
}
} // namespace

extern "C" {
EMSCRIPTEN_KEEPALIVE int afterglow_worklet_gate_create() {
    if (gStatus.load(std::memory_order_acquire) != 0) return -1;
    EmscriptenWebAudioCreateAttributes attributes{};
    attributes.latencyHint = "interactive";
    attributes.sampleRate = 48'000;
    attributes.renderSizeHint = AUDIO_CONTEXT_RENDER_SIZE_DEFAULT;
    gAudioContext = emscripten_create_audio_context(&attributes);
    if (gAudioContext == 0) return -2;
    gStatus.store(1, std::memory_order_release);
    emscripten_start_wasm_audio_worklet_thread_async(
        gAudioContext, gAudioStack, sizeof(gAudioStack), audioThreadStarted, nullptr);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int afterglow_worklet_gate_start_simulation() {
    if (gStatus.load(std::memory_order_acquire) != 3) return -1;
    bool expected = false;
    if (!gSimulationStarted.compare_exchange_strong(
            expected, true, std::memory_order_acq_rel, std::memory_order_acquire))
        return -2;
    gSimulationStop.store(false, std::memory_order_release);
    gSimulationThread = std::thread(runSimulationWorker);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int afterglow_worklet_gate_stop_simulation() {
    if (!gSimulationStarted.load(std::memory_order_acquire)) return 0;
    gSimulationStop.store(true, std::memory_order_release);
    if (gSimulationThread.joinable()) gSimulationThread.join();
    gSimulationStarted.store(false, std::memory_order_release);
    return 0;
}

EMSCRIPTEN_KEEPALIVE int afterglow_worklet_gate_resume() {
    if (gStatus.load(std::memory_order_acquire) != 3 || gAudioContext == 0) return -1;
    emscripten_resume_audio_context_sync(gAudioContext);
    return emscripten_audio_context_state(gAudioContext) == AUDIO_CONTEXT_STATE_RUNNING ? 0 : -2;
}

EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_status() {
    return gStatus.load(std::memory_order_acquire);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_callbacks() {
    return gCallbacks.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_errors() {
    return gErrors.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_max_micros() {
    return gMaxMicros.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_over_budget() {
    return gOverBudget.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_max_gap_micros() {
    return gMaxGapMicros.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_simulation_updates() {
    return gSimulationUpdates.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_reflection_updates() {
    return gReflectionSimulationUpdates.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_simulation_errors() {
    return gSimulationErrors.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_simulation_max_micros() {
    return gSimulationMaxMicros.load(std::memory_order_relaxed);
}
EMSCRIPTEN_KEEPALIVE std::uint32_t afterglow_worklet_gate_simulation_running() {
    return gSimulationRunning.load(std::memory_order_acquire) ? 1u : 0u;
}
EMSCRIPTEN_KEEPALIVE float afterglow_worklet_gate_energy() { return loadFloat(gEnergyBits); }
EMSCRIPTEN_KEEPALIVE float afterglow_worklet_gate_peak() { return loadFloat(gPeakBits); }
}
