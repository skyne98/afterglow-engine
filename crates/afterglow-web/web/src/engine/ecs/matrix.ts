// Raw matrix math — the hot path.
//
// These functions compose 4x4 matrices directly into Float32Arrays without
// any Three.js object allocation or method calls. This is the batched approach
// that benchmarks at 1.4ms for 100K entities (vs 39ms for Object3D sync).

import type { TransformStore } from './components.ts';
import type { EntityId } from '../core/types.ts';

/**
 * Compose a TRS (translation, rotation, scale) into a 4x4 column-major matrix,
 * written directly into `output` at `offset`.
 *
 * This is the exact equivalent of `THREE.Matrix4.compose()` but with zero
 * object allocation and zero method calls — one tight loop of raw float math.
 */
// @hot-no-alloc-begin composeTransformInto
export function composeTransformInto(
  output: Float32Array,
  offset: number,
  transform: TransformStore,
  entity: EntityId,
): void {
  const qx = transform.rotationX[entity];
  const qy = transform.rotationY[entity];
  const qz = transform.rotationZ[entity];
  const qw = transform.rotationW[entity];

  const x2 = qx + qx;
  const y2 = qy + qy;
  const z2 = qz + qz;

  const xx = qx * x2;
  const xy = qx * y2;
  const xz = qx * z2;
  const yy = qy * y2;
  const yz = qy * z2;
  const zz = qz * z2;
  const wx = qw * x2;
  const wy = qw * y2;
  const wz = qw * z2;

  const sx = transform.scaleX[entity];
  const sy = transform.scaleY[entity];
  const sz = transform.scaleZ[entity];

  output[offset]      = (1 - (yy + zz)) * sx;
  output[offset + 1]  = (xy + wz) * sx;
  output[offset + 2]  = (xz - wy) * sx;
  output[offset + 3]  = 0;

  output[offset + 4]  = (xy - wz) * sy;
  output[offset + 5]  = (1 - (xx + zz)) * sy;
  output[offset + 6]  = (yz + wx) * sy;
  output[offset + 7]  = 0;

  output[offset + 8]  = (xz + wy) * sz;
  output[offset + 9]  = (yz - wx) * sz;
  output[offset + 10] = (1 - (xx + yy)) * sz;
  output[offset + 11] = 0;

  output[offset + 12] = transform.positionX[entity];
  output[offset + 13] = transform.positionY[entity];
  output[offset + 14] = transform.positionZ[entity];
  output[offset + 15] = 1;
}
// @hot-no-alloc-end composeTransformInto

/**
 * Multiply two 4x4 column-major matrices: output = left × right.
 * All offsets are in Float32 elements (not bytes).
 */
// @hot-no-alloc-begin multiplyMatricesInto
export function multiplyMatricesInto(
  output: Float32Array,
  outputOffset: number,
  left: Float32Array,
  leftOffset: number,
  right: Float32Array,
  rightOffset: number,
): void {
  const a11 = left[leftOffset];
  const a12 = left[leftOffset + 4];
  const a13 = left[leftOffset + 8];
  const a14 = left[leftOffset + 12];
  const a21 = left[leftOffset + 1];
  const a22 = left[leftOffset + 5];
  const a23 = left[leftOffset + 9];
  const a24 = left[leftOffset + 13];
  const a31 = left[leftOffset + 2];
  const a32 = left[leftOffset + 6];
  const a33 = left[leftOffset + 10];
  const a34 = left[leftOffset + 14];
  const a41 = left[leftOffset + 3];
  const a42 = left[leftOffset + 7];
  const a43 = left[leftOffset + 11];
  const a44 = left[leftOffset + 15];

  const b11 = right[rightOffset];
  const b12 = right[rightOffset + 4];
  const b13 = right[rightOffset + 8];
  const b14 = right[rightOffset + 12];
  const b21 = right[rightOffset + 1];
  const b22 = right[rightOffset + 5];
  const b23 = right[rightOffset + 9];
  const b24 = right[rightOffset + 13];
  const b31 = right[rightOffset + 2];
  const b32 = right[rightOffset + 6];
  const b33 = right[rightOffset + 10];
  const b34 = right[rightOffset + 14];
  const b41 = right[rightOffset + 3];
  const b42 = right[rightOffset + 7];
  const b43 = right[rightOffset + 11];
  const b44 = right[rightOffset + 15];

  output[outputOffset]      = a11 * b11 + a12 * b21 + a13 * b31 + a14 * b41;
  output[outputOffset + 4]  = a11 * b12 + a12 * b22 + a13 * b32 + a14 * b42;
  output[outputOffset + 8]  = a11 * b13 + a12 * b23 + a13 * b33 + a14 * b43;
  output[outputOffset + 12] = a11 * b14 + a12 * b24 + a13 * b34 + a14 * b44;

  output[outputOffset + 1]  = a21 * b11 + a22 * b21 + a23 * b31 + a24 * b41;
  output[outputOffset + 5]  = a21 * b12 + a22 * b22 + a23 * b32 + a24 * b42;
  output[outputOffset + 9]  = a21 * b13 + a22 * b23 + a23 * b33 + a24 * b43;
  output[outputOffset + 13] = a21 * b14 + a22 * b24 + a23 * b34 + a24 * b44;

  output[outputOffset + 2]  = a31 * b11 + a32 * b21 + a33 * b31 + a34 * b41;
  output[outputOffset + 6]  = a31 * b12 + a32 * b22 + a33 * b32 + a34 * b42;
  output[outputOffset + 10] = a31 * b13 + a32 * b23 + a33 * b33 + a34 * b43;
  output[outputOffset + 14] = a31 * b14 + a32 * b24 + a33 * b34 + a34 * b44;

  output[outputOffset + 3]  = a41 * b11 + a42 * b21 + a43 * b31 + a44 * b41;
  output[outputOffset + 7]  = a41 * b12 + a42 * b22 + a43 * b32 + a44 * b42;
  output[outputOffset + 11] = a41 * b13 + a42 * b23 + a43 * b33 + a44 * b43;
  output[outputOffset + 15] = a41 * b14 + a42 * b24 + a43 * b34 + a44 * b44;
}
// @hot-no-alloc-end multiplyMatricesInto
