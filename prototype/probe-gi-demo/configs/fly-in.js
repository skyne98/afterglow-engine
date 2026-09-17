// Flying inside the room at 6 m/s with consecutive captures: the reporter's case, isolated
// from the exterior. Configs/fly-in-noupdate.js is the same path with the probe update
// disabled, so the difference between the two frame-to-frame sequences is the field's.
globalThis.__probeGiConfig = {
  scene: 'complex', autoFly: 6.0, captureFrames: Array.from({ length: 15 }, (_, i) => 20 + i),
};
await import('../demo.js');
