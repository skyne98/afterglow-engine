// Web Worker: simulates a physics worker. Writes frames to the shared
// ring buffer in wasm linear memory (SharedArrayBuffer).
//
// The worker receives { module, memory } via postMessage and instantiates
// the same wasm module with the same shared memory.

let wasm = null;
let ringBufferOffset = 0;
let ringBufferSize = 0;

self.onmessage = async (e) => {
  if (e.data.type === 'init') {
    const { module, memory } = e.data;
    const instance = await WebAssembly.instantiate(module, {
      env: { memory }
    });
    wasm = instance.exports;

    // Initialize the ring buffer header
    wasm.init_ring_buffer();

    ringBufferOffset = wasm.get_ring_buffer_ptr();
    ringBufferSize = wasm.get_ring_buffer_size();

    console.log('[worker] ring buffer at offset', ringBufferOffset, 'size', ringBufferSize);

    // Start pushing frames
    let frame = 0;
    const interval = setInterval(() => {
      // Create a fake physics payload: 64 bytes
      const payload = new Uint8Array(64);
      payload[0] = 0xDE;
      payload[1] = 0xAD;
      payload[2] = 0xBE;
      payload[3] = 0xEF;
      payload[4] = frame & 0xFF;

      // Allocate a temporary buffer in wasm memory and write the frame.
      // Since we share the memory, we can write directly via a Uint8Array view.
      const view = new Uint8Array(memory.buffer, ringBufferOffset, ringBufferSize);

      // Use the wasm write_frame function (it handles the ring buffer logic)
      // We need a pointer to the payload data in wasm memory.
      // Since the ring buffer IS in wasm memory, we can write directly.
      // But write_frame expects a pointer — let's use the data area of the ring buffer
      // as scratch space for the payload.
      // Actually, let's just write the payload bytes to a temporary location
      // and call write_frame with that pointer.

      // Simple approach: write payload to the end of the ring buffer data area
      // as scratch space (it won't interfere with the ring buffer data which
      // starts at offset 12 and wraps around).
      // Better: allocate via wasm malloc if available, or use a fixed scratch area.

      // For now, use a fixed scratch area at the end of the ring buffer.
      const scratchOffset = ringBufferOffset + ringBufferSize - 256;
      const scratchView = new Uint8Array(memory.buffer, scratchOffset, 256);
      scratchView.set(payload);

      const ret = wasm.write_frame(scratchOffset, payload.length);
      if (ret === 0) {
        frame++;
        if (frame <= 3 || frame % 60 === 0) {
          console.log('[worker] wrote frame', frame, 'first bytes:', payload[0].toString(16), payload[1].toString(16), payload[2].toString(16), payload[3].toString(16));
        }
      } else if (ret === -1) {
        // Buffer full — skip this frame
      }
    }, 16); // ~60 FPS

    // Stop after 600 frames
    setTimeout(() => {
      clearInterval(interval);
      console.log('[worker] finished, wrote', frame, 'frames');
      self.postMessage({ type: 'done', frames: frame });
    }, 600 * 16 + 1000);

  }
};
