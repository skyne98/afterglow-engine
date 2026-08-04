#!/usr/bin/env bun
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { HairFitRuntime, type HairFitDocument } from '../src/hair-fit.ts';

interface Glb {
  json: any;
  bin: Buffer;
}

function loadFaceTargetNames(): Set<string> {
  const functionalDir = join(import.meta.dir, '..', '..', '..', 'assets', 'character-rig', 'downloads', 'functional');
  const names = new Set<string>();
  const packs = [
    ['faceunits01', 'faceunits'],
    ['visemes01', 'visemes'],
    ['visemes02', 'visemes'],
  ] as const;
  for (const [pack, directory] of packs) {
    const manifest = JSON.parse(readFileSync(join(functionalDir, 'packs', `${pack}.json`), 'utf8')) as Record<string, { license: string }>;
    for (const [name, metadata] of Object.entries(manifest)) {
      if (metadata.license !== 'CC0') throw new Error(`${pack}/${name}: non-CC0 face target`);
      const bytes = readFileSync(join(functionalDir, 'targets', directory, `${name}.target`));
      if (bytes.length > 0) names.add(name);
    }
  }
  if (names.size !== 87) throw new Error(`incorrect non-empty face-pack count (${names.size})`);
  return names;
}

const faceTargetNames = loadFaceTargetNames();

function loadGlb(path: string): Glb {
  const bytes = readFileSync(path);
  if (bytes.readUInt32LE(0) !== 0x46546c67 || bytes.readUInt32LE(4) !== 2) {
    throw new Error(`${path}: incorrect GLB header`);
  }

  const jsonLength = bytes.readUInt32LE(12);
  const json = JSON.parse(bytes.toString('utf8', 20, 20 + jsonLength));
  let offset = (20 + jsonLength + 3) & ~3;
  let bin: Buffer | undefined;
  while (offset < bytes.length) {
    const length = bytes.readUInt32LE(offset);
    const type = bytes.readUInt32LE(offset + 4);
    if (type === 0x004e4942) bin = bytes.subarray(offset + 8, offset + 8 + length);
    offset += 8 + length;
  }
  if (!bin) throw new Error(`${path}: no BIN chunk`);
  return { json, bin };
}

function readVec3(glb: Glb, accessorIndex: number): Float32Array {
  const { json, bin } = glb;
  const accessor = json.accessors[accessorIndex];
  if (accessor.componentType !== 5126 || accessor.type !== 'VEC3') {
    throw new Error(`accessor ${accessorIndex}: expected float VEC3`);
  }

  const output = new Float32Array(accessor.count * 3);
  if (accessor.bufferView !== undefined) {
    const view = json.bufferViews[accessor.bufferView];
    const base = (view.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
    const stride = view.byteStride ?? 12;
    for (let index = 0; index < accessor.count; index++) {
      for (let component = 0; component < 3; component++) {
        output[index * 3 + component] = bin.readFloatLE(base + index * stride + component * 4);
      }
    }
  }

  const sparse = accessor.sparse;
  if (!sparse) return output;
  const indexView = json.bufferViews[sparse.indices.bufferView];
  const valueView = json.bufferViews[sparse.values.bufferView];
  const indexOffset = (indexView.byteOffset ?? 0) + (sparse.indices.byteOffset ?? 0);
  const valueOffset = (valueView.byteOffset ?? 0) + (sparse.values.byteOffset ?? 0);
  const indexSize = sparse.indices.componentType === 5121 ? 1 : sparse.indices.componentType === 5123 ? 2 : 4;
  const readIndex = (offset: number): number => {
    if (sparse.indices.componentType === 5121) return bin.readUInt8(offset);
    if (sparse.indices.componentType === 5123) return bin.readUInt16LE(offset);
    if (sparse.indices.componentType === 5125) return bin.readUInt32LE(offset);
    throw new Error(`accessor ${accessorIndex}: incorrect sparse index type`);
  };

  for (let entry = 0; entry < sparse.count; entry++) {
    const index = readIndex(indexOffset + entry * indexSize);
    for (let component = 0; component < 3; component++) {
      output[index * 3 + component] = bin.readFloatLE(valueOffset + (entry * 3 + component) * 4);
    }
  }
  return output;
}

function countTransparentColors(glb: Glb, accessorIndex: number): number {
  const { json, bin } = glb;
  const accessor = json.accessors[accessorIndex];
  if (accessor.type !== 'VEC4' || accessor.bufferView === undefined) {
    throw new Error(`accessor ${accessorIndex}: expected color VEC4`);
  }
  const view = json.bufferViews[accessor.bufferView];
  const componentSize = accessor.componentType === 5121 ? 1 : accessor.componentType === 5123 ? 2 : 4;
  const stride = view.byteStride ?? componentSize * 4;
  const base = (view.byteOffset ?? 0) + (accessor.byteOffset ?? 0) + componentSize * 3;
  let transparent = 0;
  for (let vertex = 0; vertex < accessor.count; vertex++) {
    const offset = base + vertex * stride;
    const alpha = accessor.componentType === 5121
      ? bin.readUInt8(offset) / 0xff
      : accessor.componentType === 5123
        ? bin.readUInt16LE(offset) / 0xffff
        : bin.readFloatLE(offset);
    if (alpha < 0.5) transparent++;
  }
  return transparent;
}

function maximumDisplacement(values: Float32Array, label: string): number {
  let maximum = 0;
  for (let offset = 0; offset < values.length; offset += 3) {
    const distance = Math.hypot(values[offset], values[offset + 1], values[offset + 2]);
    if (!Number.isFinite(distance)) throw new Error(`${label}: non-finite morph data`);
    maximum = Math.max(maximum, distance);
  }
  return maximum;
}

function validateBody(sex: 'male' | 'female'): void {
  const publicDir = join(import.meta.dir, '..', 'public');
  const path = join(publicDir, `character_${sex}.glb`);
  const names = JSON.parse(readFileSync(join(publicDir, `character_${sex}.morphs.json`), 'utf8')) as string[];
  const controls = JSON.parse(readFileSync(join(publicDir, `character_${sex}.controls.json`), 'utf8')) as Array<{
    category: string;
    label: string;
    negative: string;
    positive: string;
  }>;
  const hairFitDocument = JSON.parse(
    readFileSync(join(publicDir, `character_${sex}.hair-fit.json`), 'utf8'),
  ) as HairFitDocument;
  const glb = loadGlb(path);
  const bodyMeshIndex = glb.json.meshes.findIndex((mesh: any) => mesh.primitives[0].targets?.length > 0);
  if (bodyMeshIndex < 0) throw new Error(`${sex}: no body morph mesh`);
  const primitive = glb.json.meshes[bodyMeshIndex].primitives[0];
  const targets = primitive.targets as Array<{ POSITION: number; NORMAL?: number }>;
  const expectedMorphs = sex === 'male' ? 691 : 689;
  const expectedControls = sex === 'male' ? 423 : 422;
  const vertices = glb.json.accessors[primitive.attributes.POSITION].count as number;

  if (targets.length !== expectedMorphs || names.length !== targets.length) {
    throw new Error(`${sex}: incorrect morph count or sidecar count`);
  }
  if (controls.length !== expectedControls) throw new Error(`${sex}: incorrect control count`);
  const controlledNames = new Set(controls.flatMap((control) => [control.negative, control.positive]).filter(Boolean));
  const missingTargets = [...controlledNames].filter((name) => !names.includes(name));
  const uncontrolledTargets = names.filter((name) => !controlledNames.has(name) && !name.startsWith('asian-') && !name.startsWith('african-'));
  if (missingTargets.length || uncontrolledTargets.length) {
    throw new Error(`${sex}: control coverage error`);
  }
  if (vertices > 25_000) throw new Error(`${sex}: flat polygon split returned (${vertices} vertices)`);
  if (primitive.attributes.COLOR_0 === undefined) throw new Error(`${sex}: face-helper colors are missing`);
  const transparentScalpVertices = countTransparentColors(glb, primitive.attributes.COLOR_0);
  if (transparentScalpVertices < 100 || transparentScalpVertices > 2_000) {
    throw new Error(`${sex}: incorrect proxy scalp mask (${transparentScalpVertices})`);
  }
  const faceCategories = new Map<string, number>();
  for (const control of controls) {
    if (control.category.startsWith('expression-') || control.category.startsWith('speech-')) {
      faceCategories.set(control.category, (faceCategories.get(control.category) ?? 0) + 1);
    }
  }
  const faceControlCount = [...faceCategories.values()].reduce((sum, count) => sum + count, 0);
  if (faceControlCount !== 87 || faceCategories.get('speech-microsoft') !== 21 || faceCategories.get('speech-meta') !== 14) {
    throw new Error(`${sex}: incorrect face-control set`);
  }
  for (const required of faceTargetNames) {
    if (!names.includes(required)) throw new Error(`${sex}: missing face target ${required}`);
  }
  if (names.includes('sil_00') || names.includes('viseme_sil')) {
    throw new Error(`${sex}: a zero-displacement silence label became a morph`);
  }
  if (targets.some((target) => target.NORMAL !== undefined)) {
    throw new Error(`${sex}: normal morphs cause incorrect Three.js shading`);
  }

  const basePositions = readVec3(glb, primitive.attributes.POSITION);
  const maxima = targets.map((target, index) => {
    const values = readVec3(glb, target.POSITION);
    const maximum = maximumDisplacement(values, `${sex}/${names[index]}`);
    const minimum = [Infinity, Infinity, Infinity];
    const upper = [-Infinity, -Infinity, -Infinity];
    for (let offset = 0; offset < values.length; offset += 3) {
      for (let component = 0; component < 3; component++) {
        const position = basePositions[offset + component] + values[offset + component];
        minimum[component] = Math.min(minimum[component], position);
        upper[component] = Math.max(upper[component], position);
      }
    }
    const size = upper.map((value, component) => value - minimum[component]);
    if (size[0] < 0.2 || size[1] < 0.4 || size[2] < 0.1 || size.some((value) => value > 3)) {
      throw new Error(`${sex}/${names[index]}: incorrect body bounds (${size.join(', ')})`);
    }
    return maximum;
  });
  const minimum = Math.min(...maxima);
  const maximum = Math.max(...maxima);
  if (minimum <= 1e-6) throw new Error(`${sex}: a morph was erased (${minimum})`);
  if (maximum > 1.25) throw new Error(`${sex}: a morph is too large (${maximum})`);

  for (const race of ['asian', 'african']) {
    const index = names.indexOf(`${race}-${sex}-young`);
    if (index < 0 || maxima[index] < 0.01 || maxima[index] > 0.15) {
      throw new Error(`${sex}: incorrect ${race} replacement morph`);
    }
  }

  const hairFit = new HairFitRuntime(hairFitDocument, names);
  if (hairFit.styles.length !== 2 || hairFitDocument.targets['head-scale-horiz-incr'] === undefined) {
    throw new Error(`${sex}: incorrect hair-fit style or structural-target set`);
  }
  for (const faceTarget of faceTargetNames) {
    if (hairFitDocument.targets[faceTarget] !== undefined) {
      throw new Error(`${sex}: transient face target ${faceTarget} changes hair rest fit`);
    }
  }
  const scalpMeshIndex = glb.json.meshes.findIndex((mesh: any) => mesh.name === hairFit.scalp.mesh);
  if (scalpMeshIndex < 0) throw new Error(`${sex}: missing fitted scalp mesh`);
  const scalpPrimitive = glb.json.meshes[scalpMeshIndex].primitives[0];
  const scalpNode = glb.json.nodes.find((node: any) => node.mesh === scalpMeshIndex && node.skin !== undefined);
  if (!scalpNode) throw new Error(`${sex}: fitted scalp is not skinned`);
  const scalpPositions = readVec3(glb, scalpPrimitive.attributes.POSITION);
  const fittedScalp = new Float32Array(scalpPositions.length);
  hairFit.fitScalp(fittedScalp);
  let scalpError = 0;
  for (let offset = 0; offset < fittedScalp.length; offset++) {
    scalpError = Math.max(scalpError, Math.abs(fittedScalp[offset] - scalpPositions[offset]));
  }
  if (scalpError > 3e-6) throw new Error(`${sex}: neutral scalp fit error ${scalpError}`);

  for (const style of hairFit.styles) {
    const meshIndex = glb.json.meshes.findIndex((mesh: any) => mesh.name === style.mesh);
    if (meshIndex < 0) throw new Error(`${sex}: missing hair mesh ${style.mesh}`);
    const hairPrimitive = glb.json.meshes[meshIndex].primitives[0];
    if (hairPrimitive.targets?.length || hairPrimitive.attributes.TEXCOORD_0 === undefined) {
      throw new Error(`${sex}/${style.id}: incorrect hair primitive`);
    }
    const skinnedNode = glb.json.nodes.find((node: any) => node.mesh === meshIndex && node.skin !== undefined);
    if (!skinnedNode) throw new Error(`${sex}/${style.id}: hair is not skinned`);
    const positions = readVec3(glb, hairPrimitive.attributes.POSITION);
    if (positions.length !== style.vertexCount * 3) {
      throw new Error(`${sex}/${style.id}: hair vertex count differs from fit data`);
    }
    const fitted = new Float32Array(positions.length);
    hairFit.fit(style, fitted);
    let fitError = 0;
    for (let offset = 0; offset < fitted.length; offset++) {
      fitError = Math.max(fitError, Math.abs(fitted[offset] - positions[offset]));
    }
    if (fitError > 3e-6) throw new Error(`${sex}/${style.id}: neutral fit error ${fitError}`);
    const headScale = names.indexOf('head-scale-horiz-incr');
    if (headScale < 0 || !hairFit.setTarget(headScale, 1)) {
      throw new Error(`${sex}/${style.id}: no dynamic head-width fit target`);
    }
    const changed = new Float32Array(positions.length);
    hairFit.fit(style, changed);
    let maximumFitMovement = 0;
    for (let offset = 0; offset < changed.length; offset += 3) {
      maximumFitMovement = Math.max(
        maximumFitMovement,
        Math.hypot(
          changed[offset] - fitted[offset],
          changed[offset + 1] - fitted[offset + 1],
          changed[offset + 2] - fitted[offset + 2],
        ),
      );
    }
    if (maximumFitMovement < 0.005 || !hairFit.setTarget(headScale, 0)) {
      throw new Error(`${sex}/${style.id}: head-width fit did not move or reset`);
    }
    readFileSync(join(publicDir, `hair-${style.id}-diffuse.png`));
  }

  console.log(
    `${sex}: ${vertices} body vertices, ${targets.length} morphs, `
    + `${hairFit.styles.length} hair styles, displacement ${minimum.toFixed(6)}-${maximum.toFixed(6)}`,
  );
}

validateBody('male');
validateBody('female');
