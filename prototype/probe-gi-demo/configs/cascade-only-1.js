// Cascade level disagreement: indirect from cascade 1 ONLY, lit room, fixed pose. Compare the
// logged mean luma across the three: a large spread means the handover ramp is a visible
// lighting change as the camera moves a surface between cascades.
globalThis.__probeGiConfig = {
  scene: 'complex', staticLight: true, debug: 4, debugCascade: 1,
  cameras: Array.from({ length: 3 }, () => ({ p: [0.0, 1.6, -3.0], yaw: Math.PI * 0.5, pitch: 0.0 })),
};
await import('../demo.js');
