// Room, three camera heights looking at the floor ahead: the cascade-coverage step is
// camera-locked, so its on-screen position moves with eye height.
globalThis.__probeGiConfig = { scene: 'complex', cameras: [{ p: [0.0, 1.2, -1.0], yaw: 1.5708, pitch: -0.45 }, { p: [0.0, 2.4, -1.0], yaw: 1.5708, pitch: -0.45 }, { p: [0.0, 3.6, -1.0], yaw: 1.5708, pitch: -0.50 }] };
await import('../demo.js');
