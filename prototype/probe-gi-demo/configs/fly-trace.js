// Temporal-stability trace: the camera flies at 4 m/s through the room, crossing a
// 1 m cascade-0 cell every ~15 frames, so the wrapped-band reseed is a recurring event
// rather than a one-off. The light is frozen so the only thing moving the field is the
// scroll itself. `fieldTrace` logs the per-frame atlas delta; the captures are consecutive
// frames (the wrap is a one-frame event, so a spacing of 60 frames cannot show it).
globalThis.__probeGiConfig = {
  scene: 'complex',
  autoFly: 4.0,
  staticLight: true,
  fieldTrace: true,
  captureFrames: [90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101],
};
await import('../demo.js');
