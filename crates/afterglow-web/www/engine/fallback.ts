// Fallback assets — Garry's Mod-style visible "loading/missing" indicators.
//
// When an asset hasn't loaded yet, the store swaps in one of these so the
// developer immediately sees "this is loading" — not a silent gray texture.
//
// - Texture: purple/black checkerboard (the iconic Source engine missing texture)
// - Geometry: bright magenta box (GMod's "ERROR" model vibe)
// - GLTF: an empty Group with the magenta error box attached
//
// All are generated procedurally — no external files needed.

import * as THREE from 'three';

/** The classic purple/black checkerboard for missing/loading textures. */
let _fallbackTexture: THREE.Texture | null = null;

/** Bright magenta box for missing/loading geometry. */
let _fallbackGeometry: THREE.BufferGeometry | null = null;

/** Magenta error material for the fallback mesh. */
let _fallbackMaterial: THREE.Material | null = null;

/** Empty group with an error box — for missing/loading GLTF models. */
let _fallbackGroup: THREE.Group | null = null;

/**
 * Generate the purple/black checkerboard texture (GMod-style).
 * 64×64 canvas, 8×8 px squares, two tones of purple.
 */
function createCheckerboardTexture(): THREE.Texture {
  const size = 64;
  const sq = 8; // square size in px
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext('2d')!;

  for (let y = 0; y < size; y += sq) {
    for (let x = 0; x < size; x += sq) {
      const isBlack = ((x / sq) + (y / sq)) % 2 === 0;
      ctx.fillStyle = isBlack ? '#000000' : '#9b00ff';
      ctx.fillRect(x, y, sq, sq);
    }
  }

  const texture = new THREE.CanvasTexture(canvas);
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.repeat.set(4, 4); // tile so it's visible on large surfaces
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

/** Get the shared fallback texture (created once, reused). */
export function fallbackTexture(): THREE.Texture {
  if (!_fallbackTexture) _fallbackTexture = createCheckerboardTexture();
  return _fallbackTexture;
}

/** Get the shared fallback geometry — a 2×2×2 magenta box. */
export function fallbackGeometry(): THREE.BufferGeometry {
  if (!_fallbackGeometry) {
    _fallbackGeometry = new THREE.BoxGeometry(2, 2, 2);
  }
  return _fallbackGeometry;
}

/** Get the shared fallback material — bright magenta, unlit. */
export function fallbackMaterial(): THREE.Material {
  if (!_fallbackMaterial) {
    _fallbackMaterial = new THREE.MeshBasicMaterial({ color: 0xff00ff });
  }
  return _fallbackMaterial;
}

/** Get the shared fallback group — an error box with the checkerboard texture. */
export function fallbackGroup(): THREE.Group {
  if (!_fallbackGroup) {
    _fallbackGroup = new THREE.Group();
    const mesh = new THREE.Mesh(fallbackGeometry(), fallbackMaterial());
    _fallbackGroup.add(mesh);
    _fallbackGroup.name = '__afterglow_fallback__';
  }
  // Return a clone so each handle gets its own (Three.js objects aren't shareable
  // across multiple parents). Cloning is cheap — shares geometry/material.
  return _fallbackGroup.clone(true);
}

/** Dispose all shared fallback assets. */
export function disposeFallbacks(): void {
  _fallbackTexture?.dispose();
  _fallbackGeometry?.dispose();
  _fallbackMaterial?.dispose();
  _fallbackGroup = null;
}
