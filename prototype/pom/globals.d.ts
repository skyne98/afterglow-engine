// Ambient globals for the POM prototype.
//
// The engine demos load `engine-bundle.js` as a classic script that sets
// `window.THREE` (Three.js WebGPU + TSL spread on top). Authored scenes use
// those globals directly. This declaration lets tsserver type-check the scene
// without a full Three type install (matching the repo convention).
declare global {
  interface Window {
    /** Three.js WebGPU + TSL, set by engine-bundle.js. */
    THREE: any;
  }
}

export {};
