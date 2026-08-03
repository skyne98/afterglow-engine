import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
import { controlBelongsToZone, selectTriangleCategory } from './zone-utils.ts';
import {
  EXPRESSION_PRESETS,
  SPEECH_PRESETS,
  type FacePreset,
} from './expression-presets.ts';

/** Simple WebGL character editor prototype around a skinned+morphed glTF. */

interface MorphControlSpec {
  category: string;
  label: string;
  negative: string;
  positive: string;
}

interface MorphHandle {
  spec: MorphControlSpec;
  negativeIndex: number;
  positiveIndex: number;
  slider: HTMLInputElement;
  valSpan: HTMLSpanElement;
  row: HTMLElement;
}

interface ZonePaint {
  id: string;
  category: string;
  label: string;
  color: number;
  index: THREE.BufferAttribute;
}

interface MeshZoneData {
  source: THREE.SkinnedMesh;
  hitMesh: THREE.Mesh;
  triangleZones: Array<ZonePaint | undefined>;
  zones: ZonePaint[];
  hoverOverlay: THREE.SkinnedMesh;
  selectedOverlay: THREE.SkinnedMesh;
}

interface ZoneHit {
  meshData: MeshZoneData;
  zone: ZonePaint;
}

const DEFAULT_BODY = 'character_female';
const CHARACTER_ASSET_REVISION = '2026-08-03-face-controls-v7';
type Ethnic = 'caucasian' | 'asian' | 'african';

class CharacterEditor {
  private renderer!: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera!: THREE.PerspectiveCamera;
  private controls!: OrbitControls;
  private loader = new GLTFLoader();

  private root?: THREE.Group;
  private morphNames: string[] = [];
  private controlSpecs: MorphControlSpec[] = [];
  private skinnedMeshes: THREE.SkinnedMesh[] = [];
  private meshZones: MeshZoneData[] = [];
  private hoveredZone?: ZoneHit;
  private selectedZone?: ZoneHit;
  private raycaster = new THREE.Raycaster();
  private pointer = new THREE.Vector2();
  private pendingZoneX = 0;
  private pendingZoneY = 0;
  private zonePointerPending = false;
  private lastZoneSample = -Infinity;
  private bones: THREE.Bone[] = [];
  private morphHandles: MorphHandle[] = [];
  private skeletonHelper?: THREE.SkeletonHelper;

  private statusEl = document.getElementById('status')!;
  private metaEl = document.getElementById('char-meta')!;
  private morphList = document.getElementById('morph-list') as HTMLDivElement;
  private boneList = document.getElementById('bone-list') as HTMLDivElement;
  private partsList = document.getElementById('parts-list') as HTMLDivElement;
  private morphCountEl = document.getElementById('morph-count')!;
  private boneCountEl = document.getElementById('bone-count')!;
  private loadingEl = document.getElementById('loading') as HTMLDivElement;
  private fileInput = document.getElementById('file-input') as HTMLInputElement;
  private zoneSelectionEl = document.getElementById('zone-selection') as HTMLDivElement;
  private expressionPresetSelect = document.getElementById('expression-preset-select') as HTMLSelectElement;
  private speechPresetSelect = document.getElementById('speech-preset-select') as HTMLSelectElement;

  constructor() {
    this.initViewport();
    this.initUI();
    this.animate();
  }

  private initViewport(): void {
    const host = document.getElementById('viewport')!;
    this.camera = new THREE.PerspectiveCamera(50, 1, 0.1, 10000);
    this.camera.position.set(0, 1.3, 2.2);

    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.shadowMap.enabled = true;
    host.appendChild(this.renderer.domElement);
    this.renderer.domElement.addEventListener('pointermove', (event) => this.queueZoneHover(event));
    this.renderer.domElement.addEventListener('pointerleave', () => {
      this.zonePointerPending = false;
      this.clearZoneHover();
    });
    this.renderer.domElement.addEventListener('click', (event) => this.selectZoneAtPointer(event));

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.target.set(0, 1.0, 0);
    this.controls.enableDamping = true;
    this.controls.update();

    // Lights.
    const hemi = new THREE.HemisphereLight(0xffffff, 0x444455, 1.0);
    const key = new THREE.DirectionalLight(0xffffff, 2.2);
    key.position.set(2, 4, 3);
    key.castShadow = true;
    const fill = new THREE.DirectionalLight(0x99ccff, 0.5);
    fill.position.set(-2, 1, -2);
    this.scene.add(hemi, key, fill);

    // Ground grid.
    const grid = new THREE.GridHelper(6, 24, 0x2a3340, 0x1b212b);
    grid.position.y = 0;
    this.scene.add(grid);

    const resize = () => {
      const w = host.clientWidth, h = host.clientHeight;
      this.camera.aspect = w / h;
      this.camera.updateProjectionMatrix();
      this.renderer.setSize(w, h);
    };
    window.addEventListener('resize', resize);
    resize();
  }

  private initUI(): void {
    document.getElementById('load-btn')!.addEventListener('click', () => this.fileInput.click());
    this.fileInput.addEventListener('change', () => {
      const f = this.fileInput.files?.[0];
      if (f) this.loadFromUrl(URL.createObjectURL(f));
    });
    document.getElementById('randomize-btn')!.addEventListener('click', () => this.randomize());
    document.getElementById('reset-btn')!.addEventListener('click', () => this.reset());
    document.getElementById('zone-clear-btn')!.addEventListener('click', () => this.clearZoneSelection());
    document.getElementById('face-focus-btn')!.addEventListener('click', () => this.focusFace());
    document.getElementById('face-reset-btn')!.addEventListener('click', () => this.resetFace());
    this.populatePresetSelect(this.expressionPresetSelect, EXPRESSION_PRESETS);
    this.populatePresetSelect(this.speechPresetSelect, SPEECH_PRESETS);
    this.expressionPresetSelect.addEventListener('change', () => {
      this.applyFacePreset(EXPRESSION_PRESETS[this.expressionPresetSelect.selectedIndex], 'expression-');
    });
    this.speechPresetSelect.addEventListener('change', () => {
      this.applyFacePreset(SPEECH_PRESETS[this.speechPresetSelect.selectedIndex], 'speech-');
    });
    document.getElementById('male-btn')!.addEventListener('click', () => this.loadBody('character_male'));
    document.getElementById('female-btn')!.addEventListener('click', () => this.loadBody('character_female'));
    const es = document.getElementById('ethnicity-select') as HTMLSelectElement;
    es.addEventListener('change', () => this.setEthnicity(es.value as Ethnic));
    const np = document.getElementById('smooth-toggle') as HTMLInputElement;
    const nw = document.getElementById('wireframe-toggle') as HTMLInputElement;
    const ns = document.getElementById('skeleton-toggle') as HTMLInputElement;
    const applyRender = () => {
      for (const m of this.skinnedMeshes) {
        if (m.material) {
          (m.material as THREE.MeshStandardMaterial).flatShading = !np.checked;
          (m.material as THREE.MeshStandardMaterial).wireframe = nw.checked;
          (m.material as THREE.MeshStandardMaterial).needsUpdate = true;
        }
      }
    };
    np.addEventListener('change', applyRender);
    nw.addEventListener('change', applyRender);
    ns.addEventListener('change', () => this.toggleSkeleton(ns.checked));
  }

  // ------------------------------------------------------------------ loading
  /** Load a body by base name ("character") + its morph-sidecar. */
  async loadBody(base: string): Promise<void> {
    this.showStatus(`Loading ${base}.glb…`);
    const glb = `${base}.glb`;
    const assetQuery = `?v=${CHARACTER_ASSET_REVISION}`;
    try {
      await this.loadFromUrl(`${glb}${assetQuery}`);
      const [morphResponse, controlResponse] = await Promise.all([
        fetch(`${base}.morphs.json${assetQuery}`, { cache: 'no-store' }),
        fetch(`${base}.controls.json${assetQuery}`, { cache: 'no-store' }),
      ]);
      if (morphResponse.ok && controlResponse.ok) {
        const names = (await morphResponse.json()) as string[];
        const controls = (await controlResponse.json()) as MorphControlSpec[];
        this.setMorphData(names, controls);
      } else {
        this.showStatus(`${glb} loaded (no control sidecar).`);
      }
    } catch (e) {
      this.showStatus(`Could not load ${glb}: ${(e as Error).message}`);
    }
  }

  async loadDefault(): Promise<void> {
    await this.loadBody(DEFAULT_BODY);
  }

  private loadFromUrl(url: string): Promise<void> {
    this.loadingEl.style.display = 'flex';
    return new Promise((resolve, reject) => {
      this.loader.load(
        url,
        (gltf) => {
          this.setCharacter(gltf.scene);
          this.loadingEl.style.display = 'none';
          this.showStatus(`Loaded ${url.split('/').pop()}`);
          resolve();
        },
        undefined,
        (err) => {
          this.loadingEl.style.display = 'none';
          this.showStatus(`Load failed: ${err instanceof Error ? err.message : String(err)}`);
          reject(err instanceof Error ? err : new Error(String(err)));
        },
      );
    });
  }

  private setCharacter(scene: THREE.Group): void {
    // Remove any previous character (and its skeleton helper).
    this.clearZoneState();
    if (this.root) this.scene.remove(this.root);
    this.setSkeletonVisible(false);
    this.root = scene;
    scene.scale.setScalar(1);
    this.scene.add(scene);
    this.fitCameraTo(scene);

    this.collectMeshes(scene);
    this.collectBones(scene);
    this.buildMorphUI();
    this.buildBoneUI();
    this.buildPartsUI();
    this.updateMeta();
  }

  private collectMeshes(obj: THREE.Object3D): void {
    this.skinnedMeshes = [];
    obj.traverse((o) => {
      if ((o as THREE.SkinnedMesh).isSkinnedMesh) {
        const m = o as THREE.SkinnedMesh;
        // Face-helper vertex colors distinguish the eyes, teeth, and tongue.
        const vertexColors = m.geometry.getAttribute('color') !== undefined;
        m.material = new THREE.MeshStandardMaterial({
          color: vertexColors ? 0xffffff : 0xc9a27e,
          roughness: 0.6,
          vertexColors,
        });
        this.skinnedMeshes.push(m);
      }
    });
  }

  private collectBones(obj: THREE.Object3D): void {
    this.bones = [];
    obj.traverse((o) => {
      if ((o as THREE.Bone).isBone && !(o as THREE.Bone).userData.skipped) {
        this.bones.push(o as THREE.Bone);
      }
    });
  }

  private setMorphData(names: string[], controls: MorphControlSpec[]): void {
    // Keep all names so each sidecar index stays equal to its glTF target index.
    this.morphNames = names;
    this.controlSpecs = controls;
    this.buildMorphUI();
    this.buildZoneMaps();
    this.clearZoneSelection();
    this.updateMeta();
    const ethnicity = (document.getElementById('ethnicity-select') as HTMLSelectElement).value as Ethnic;
    this.setEthnicity(ethnicity);
    this.applyFacePreset(EXPRESSION_PRESETS[this.expressionPresetSelect.selectedIndex], 'expression-', false);
    this.applyFacePreset(SPEECH_PRESETS[this.speechPresetSelect.selectedIndex], 'speech-', false);
  }

  private populatePresetSelect(select: HTMLSelectElement, presets: readonly FacePreset[]): void {
    select.replaceChildren();
    for (const preset of presets) {
      const option = document.createElement('option');
      option.textContent = preset.name;
      select.appendChild(option);
    }
  }

  private applyFacePreset(preset: FacePreset, categoryPrefix: 'expression-' | 'speech-', report = true): void {
    for (const handle of this.morphHandles) {
      if (!handle.spec.category.startsWith(categoryPrefix)) continue;
      handle.slider.value = '0';
      this.applyMorphControl(handle, 0);
    }
    const missing: string[] = [];
    for (const [target, amount] of Object.entries(preset.weights)) {
      const handle = this.morphHandles.find((item) => item.spec.positive === target);
      if (!handle) {
        missing.push(target);
        continue;
      }
      handle.slider.value = String(amount);
      this.applyMorphControl(handle, amount);
    }
    if (missing.length > 0) {
      this.showStatus(`${preset.name}: missing ${missing.join(', ')}`);
    } else if (report) {
      this.showStatus(`Face preview: ${preset.name}.`);
    }
  }

  private resetFace(): void {
    this.expressionPresetSelect.selectedIndex = 0;
    this.speechPresetSelect.selectedIndex = 0;
    this.applyFacePreset(EXPRESSION_PRESETS[0], 'expression-', false);
    this.applyFacePreset(SPEECH_PRESETS[0], 'speech-', false);
    this.showStatus('Face preview reset.');
  }

  private focusFace(): void {
    const head = this.bones.find((bone) => bone.name.toLowerCase() === 'head');
    if (!head) {
      this.showStatus('The skeleton has no head bone.');
      return;
    }
    const position = new THREE.Vector3();
    head.getWorldPosition(position);
    const direction = this.camera.position.clone().sub(this.controls.target).normalize();
    this.controls.target.copy(position);
    this.camera.position.copy(position).addScaledVector(direction, 0.55);
    this.camera.near = 0.01;
    this.camera.updateProjectionMatrix();
    this.controls.update();
    this.showStatus('Face preview focused.');
  }

  private buildMorphUI(): void {
    this.morphList.innerHTML = '';
    this.morphHandles = [];
    const indexByName = new Map(this.morphNames.map((name, index) => [name, index]));
    let previousCategory = '';
    for (const spec of this.controlSpecs) {
      const negativeIndex = spec.negative ? (indexByName.get(spec.negative) ?? -1) : -1;
      const positiveIndex = spec.positive ? (indexByName.get(spec.positive) ?? -1) : -1;
      if (negativeIndex < 0 && positiveIndex < 0) continue;
      if (spec.category !== previousCategory) {
        const heading = document.createElement('div');
        heading.className = 'morph-category';
        heading.textContent = spec.category;
        this.morphList.appendChild(heading);
        previousCategory = spec.category;
      }
      const row = document.createElement('div');
      row.className = 'morph-row';
      const label = document.createElement('label');
      label.title = `negative: ${spec.negative || 'none'}\npositive: ${spec.positive || 'none'}`;
      label.textContent = spec.label;
      const slider = document.createElement('input');
      slider.type = 'range';
      slider.min = spec.negative ? '-1' : '0';
      slider.max = '1';
      slider.step = '0.001';
      slider.value = '0';
      const val = document.createElement('span');
      val.className = 'val';
      val.textContent = '0.00';
      row.append(label, slider, val);
      this.morphList.appendChild(row);
      const handle = { spec, negativeIndex, positiveIndex, slider, valSpan: val, row };
      this.morphHandles.push(handle);
      slider.addEventListener('input', () => this.applyMorphControl(handle, parseFloat(slider.value)));
    }
    this.morphCountEl.textContent = String(this.morphHandles.length);
  }

  private applyMorphIndex(index: number, amount: number): void {
    if (index < 0) return;
    for (const mesh of this.skinnedMeshes) {
      const influences = mesh.morphTargetInfluences;
      if (influences && index < influences.length) influences[index] = amount;
    }
  }

  private applyMorphControl(handle: MorphHandle, amount: number): void {
    this.applyMorphIndex(handle.negativeIndex, Math.max(-amount, 0));
    this.applyMorphIndex(handle.positiveIndex, Math.max(amount, 0));
    handle.valSpan.textContent = amount.toFixed(2);
  }

  /** Drive the transferred ethnicity morphs (asian/african; caucasian = baseline). */
  private setEthnicity(e: Ethnic): void {
    const asian = this.morphNames.findIndex((n) => n.startsWith('asian-'));
    const african = this.morphNames.findIndex((n) => n.startsWith('african-'));
    this.applyMorphIndex(asian, e === 'asian' ? 1 : 0);
    this.applyMorphIndex(african, e === 'african' ? 1 : 0);
  }

  // --------------------------------------------------------------- body zones
  private buildZoneMaps(): void {
    this.clearZoneState();
    if (this.morphNames.length === 0 || this.controlSpecs.length === 0) return;

    const excluded = new Set(['macro', 'asymmetry']);
    const categories = [...new Set(this.controlSpecs.map((spec) => spec.category).filter((category) => !excluded.has(category)))];
    const categoryByTarget = new Map<string, number>();
    for (const spec of this.controlSpecs) {
      const category = categories.indexOf(spec.category);
      if (category < 0) continue;
      if (spec.negative) categoryByTarget.set(spec.negative, category);
      if (spec.positive) categoryByTarget.set(spec.positive, category);
    }
    const bilateral = new Set(this.controlSpecs.filter((spec) => spec.label.endsWith('(left)')).map((spec) => spec.category));
    const indexByName = new Map(this.morphNames.map((name, index) => [name, index]));

    for (const source of this.skinnedMeshes) {
      const positions = source.geometry.getAttribute('position');
      const index = source.geometry.getIndex();
      const targets = source.geometry.morphAttributes.position;
      if (!positions || !index || !targets) continue;

      const scores = categories.map(() => new Float32Array(positions.count));
      for (const [targetName, category] of categoryByTarget) {
        if (targetName.startsWith('measure-')) continue;
        const targetIndex = indexByName.get(targetName);
        if (targetIndex === undefined) continue;
        const target = targets[targetIndex];
        if (!target) continue;
        let maximum = 0;
        for (let vertex = 0; vertex < target.count; vertex++) {
          maximum = Math.max(maximum, Math.hypot(target.getX(vertex), target.getY(vertex), target.getZ(vertex)));
        }
        if (maximum <= 1e-6) continue;
        const threshold = maximum * 0.08;
        const categoryScores = scores[category];
        for (let vertex = 0; vertex < target.count; vertex++) {
          const displacement = Math.hypot(target.getX(vertex), target.getY(vertex), target.getZ(vertex));
          if (displacement < threshold) continue;
          const score = displacement / maximum;
          if (score > categoryScores[vertex]) categoryScores[vertex] = score;
        }
      }

      const bestCategory = new Int16Array(positions.count);
      bestCategory.fill(-1);
      const bestScore = new Float32Array(positions.count);
      for (let category = 0; category < scores.length; category++) {
        const categoryScores = scores[category];
        for (let vertex = 0; vertex < positions.count; vertex++) {
          if (categoryScores[vertex] > bestScore[vertex]) {
            bestScore[vertex] = categoryScores[vertex];
            bestCategory[vertex] = category;
          }
        }
      }

      const triangleZoneIds: Array<string | undefined> = new Array(index.count / 3);
      const zoneIndices = new Map<string, number[]>();
      const zoneInfo = new Map<string, { category: string; label: string }>();
      for (let triangle = 0; triangle < index.count / 3; triangle++) {
        const a = index.getX(triangle * 3);
        const b = index.getX(triangle * 3 + 1);
        const c = index.getX(triangle * 3 + 2);
        const ca = bestCategory[a], cb = bestCategory[b], cc = bestCategory[c];
        const categoryIndex = selectTriangleCategory(
          [ca, cb, cc],
          [bestScore[a], bestScore[b], bestScore[c]],
        );
        if (categoryIndex < 0) continue;
        const category = categories[categoryIndex];
        let side = '';
        if (bilateral.has(category)) {
          const centerX = (positions.getX(a) + positions.getX(b) + positions.getX(c)) / 3;
          side = centerX >= 0 ? 'left' : 'right';
        }
        const zoneId = side ? `${category}:${side}` : category;
        const label = side ? `${category} (${side})` : category;
        triangleZoneIds[triangle] = zoneId;
        let zone = zoneIndices.get(zoneId);
        if (!zone) {
          zone = [];
          zoneIndices.set(zoneId, zone);
          zoneInfo.set(zoneId, { category, label });
        }
        zone.push(a, b, c);
      }

      const palette = [
        0xff6b6b, 0x4ecdc4, 0xffd166, 0x6c5ce7, 0x45b7d1, 0xf78fb3,
        0x95e06c, 0xff9f43, 0x54a0ff, 0xc56cf0, 0x00d2d3, 0xff6b81,
        0xa3cb38, 0xffc048, 0x18dcff, 0x7d5fff, 0x32ff7e, 0xff4d4d,
        0x7efff5, 0xe056fd, 0xfffa65, 0xcd84f1,
      ];
      const paintById = new Map<string, ZonePaint>();
      const zones: ZonePaint[] = [];
      let colorIndex = 0;
      for (const [id, values] of zoneIndices) {
        const info = zoneInfo.get(id)!;
        const paint = {
          id,
          category: info.category,
          label: info.label,
          color: palette[colorIndex++ % palette.length],
          index: new THREE.Uint32BufferAttribute(values, 1),
        };
        zones.push(paint);
        paintById.set(id, paint);
      }
      const triangleZones = triangleZoneIds.map((id) => id ? paintById.get(id) : undefined);
      const hitMesh = this.createZoneHitMesh(source);
      const hoverOverlay = this.createZoneOverlay(source, 0.5);
      const selectedOverlay = this.createZoneOverlay(source, 0.3);
      this.meshZones.push({ source, hitMesh, triangleZones, zones, hoverOverlay, selectedOverlay });
    }
  }

  private createZoneHitMesh(source: THREE.SkinnedMesh): THREE.Mesh {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', source.geometry.getAttribute('position'));
    geometry.setIndex(source.geometry.getIndex());
    geometry.computeBoundingSphere();
    const mesh = new THREE.Mesh(geometry, new THREE.MeshBasicMaterial());
    mesh.matrixAutoUpdate = false;
    return mesh;
  }

  private createZoneOverlay(source: THREE.SkinnedMesh, opacity: number): THREE.SkinnedMesh {
    const geometry = new THREE.BufferGeometry();
    for (const [name, attribute] of Object.entries(source.geometry.attributes)) geometry.setAttribute(name, attribute);
    geometry.morphAttributes = source.geometry.morphAttributes;
    geometry.morphTargetsRelative = source.geometry.morphTargetsRelative;
    geometry.setIndex(new THREE.Uint32BufferAttribute([], 1));
    const material = new THREE.MeshBasicMaterial({
      color: 0xffffff,
      transparent: true,
      opacity,
      depthWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: -2,
      side: THREE.DoubleSide,
    });
    const overlay = new THREE.SkinnedMesh(geometry, material);
    overlay.name = `${source.name}-zone-overlay`;
    overlay.bindMode = source.bindMode;
    overlay.bind(source.skeleton, source.bindMatrix);
    overlay.bindMatrixInverse.copy(source.bindMatrixInverse);
    overlay.morphTargetInfluences = source.morphTargetInfluences;
    overlay.morphTargetDictionary = source.morphTargetDictionary;
    overlay.position.copy(source.position);
    overlay.quaternion.copy(source.quaternion);
    overlay.scale.copy(source.scale);
    overlay.visible = false;
    overlay.frustumCulled = false;
    overlay.renderOrder = 10;
    source.parent?.add(overlay);
    return overlay;
  }

  private zoneAtPoint(clientX: number, clientY: number): ZoneHit | undefined {
    const canvas = this.renderer.domElement;
    const rect = canvas.getBoundingClientRect();
    this.pointer.set(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -((clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.raycaster.setFromCamera(this.pointer, this.camera);
    for (const data of this.meshZones) {
      data.source.updateWorldMatrix(true, false);
      data.hitMesh.matrixWorld.copy(data.source.matrixWorld);
    }
    const intersection = this.raycaster.intersectObjects(this.meshZones.map((data) => data.hitMesh), false)[0];
    if (!intersection || intersection.faceIndex == null) return undefined;
    const meshData = this.meshZones.find((data) => data.hitMesh === intersection.object);
    const zone = meshData?.triangleZones[intersection.faceIndex];
    return meshData && zone ? { meshData, zone } : undefined;
  }

  private queueZoneHover(event: PointerEvent): void {
    if (event.buttons !== 0) {
      this.zonePointerPending = false;
      this.clearZoneHover();
      return;
    }
    this.pendingZoneX = event.clientX;
    this.pendingZoneY = event.clientY;
    this.zonePointerPending = true;
  }

  private updateZoneHover(): void {
    const hit = this.zoneAtPoint(this.pendingZoneX, this.pendingZoneY);
    if (hit?.zone.id === this.hoveredZone?.zone.id && hit?.meshData === this.hoveredZone?.meshData) return;
    this.clearZoneHover();
    if (!hit) return;
    this.hoveredZone = hit;
    if (hit.zone.id !== this.selectedZone?.zone.id || hit.meshData !== this.selectedZone?.meshData) {
      hit.meshData.hoverOverlay.geometry.setIndex(hit.zone.index);
      (hit.meshData.hoverOverlay.material as THREE.MeshBasicMaterial).color.setHex(hit.zone.color);
      hit.meshData.hoverOverlay.visible = true;
    }
    this.renderer.domElement.style.cursor = 'pointer';
    if (!this.selectedZone) this.setZoneSelectionText(`Hover: ${hit.zone.label}`, hit.zone.color);
  }

  private clearZoneHover(): void {
    for (const data of this.meshZones) data.hoverOverlay.visible = false;
    this.hoveredZone = undefined;
    this.renderer.domElement.style.cursor = '';
    if (!this.selectedZone) this.setZoneSelectionText('Hover or click a body zone.');
  }

  private selectZoneAtPointer(event: PointerEvent): void {
    this.zonePointerPending = false;
    const hit = this.zoneAtPoint(event.clientX, event.clientY);
    if (!hit) return;
    for (const data of this.meshZones) data.selectedOverlay.visible = false;
    this.selectedZone = hit;
    hit.meshData.selectedOverlay.geometry.setIndex(hit.zone.index);
    (hit.meshData.selectedOverlay.material as THREE.MeshBasicMaterial).color.setHex(hit.zone.color);
    hit.meshData.selectedOverlay.visible = true;
    hit.meshData.hoverOverlay.visible = false;
    this.setZoneSelectionText(`Selected: ${hit.zone.label}`, hit.zone.color);
    this.filterMorphControls(hit.zone.category);
    this.showStatus(`Selected ${hit.zone.label} zone.`);
  }

  private clearZoneSelection(): void {
    for (const data of this.meshZones) data.selectedOverlay.visible = false;
    this.selectedZone = undefined;
    this.setZoneSelectionText('Hover or click a body zone.');
    this.filterMorphControls();
  }

  private clearZoneState(): void {
    for (const data of this.meshZones) {
      data.hitMesh.geometry.dispose();
      (data.hitMesh.material as THREE.Material).dispose();
      for (const overlay of [data.hoverOverlay, data.selectedOverlay]) {
        overlay.parent?.remove(overlay);
        overlay.geometry.dispose();
        (overlay.material as THREE.Material).dispose();
      }
    }
    this.zonePointerPending = false;
    this.meshZones = [];
    this.hoveredZone = undefined;
    this.selectedZone = undefined;
    if (this.renderer) this.renderer.domElement.style.cursor = '';
  }

  private setZoneSelectionText(text: string, color?: number): void {
    const span = this.zoneSelectionEl.querySelector('span')!;
    span.textContent = text;
    span.style.color = color === undefined ? '' : `#${color.toString(16).padStart(6, '0')}`;
  }

  private filterMorphControls(category?: string): void {
    let visible = 0;
    for (const handle of this.morphHandles) {
      const show = !category || controlBelongsToZone(handle.spec, category);
      handle.row.style.display = show ? '' : 'none';
      if (show) visible++;
    }
    for (const heading of Array.from(this.morphList.querySelectorAll<HTMLElement>('.morph-category'))) {
      const headingCategory = heading.textContent ?? '';
      const hasVisible = this.morphHandles.some((handle) => handle.spec.category === headingCategory && handle.row.style.display !== 'none');
      heading.style.display = hasVisible ? '' : 'none';
    }
    this.morphCountEl.textContent = category ? `${visible}/${this.morphHandles.length}` : String(this.morphHandles.length);
  }

  private buildBoneUI(): void {
    this.boneList.innerHTML = '';
    this.boneCountEl.textContent = String(this.bones.length);
    for (const bone of this.bones) {
      const row = document.createElement('div');
      row.className = 'bone-row';
      const label = document.createElement('label');
      label.title = (bone.name || 'bone') + '\nposition: ' + bone.position.toArray().map(n => n.toFixed(3)).join(', ');
      label.textContent = bone.name || 'bone';
      const sel = document.createElement('button');
      sel.textContent = 'Focus';
      sel.addEventListener('click', () => this.focusBone(bone));
      row.append(label, sel);
      this.boneList.appendChild(row);
    }
  }

  private focusBone(bone: THREE.Bone): void {
    const pos = new THREE.Vector3();
    bone.getWorldPosition(pos);
    this.controls.target.copy(pos);
  }

  private buildPartsUI(): void {
    this.partsList.innerHTML = '';
    const seen = new Set<string>();
    for (const m of this.skinnedMeshes) {
      const name = m.name || 'mesh';
      if (seen.has(name)) continue;
      seen.add(name);
      const row = document.createElement('div');
      row.className = 'part-row';
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = true;
      const label = document.createElement('label');
      label.textContent = name;
      row.append(cb, label);
      cb.addEventListener('change', () => { m.visible = cb.checked; });
      this.partsList.appendChild(row);
    }
    if (this.skinnedMeshes.length === 0) {
      this.partsList.innerHTML = '<div class="muted">No skinned meshes.</div>';
    }
  }

  private updateMeta(): void {
    const tri = this.skinnedMeshes.reduce((sum, m) => sum + (m.geometry.index ? m.geometry.index.count / 3 : (m.geometry.attributes.position?.count ?? 0) / 3), 0);
    this.metaEl.textContent = [
      `Meshes: ${this.skinnedMeshes.length}`,
      `Bones: ${this.bones.length}`,
      `Morphs: ${this.morphHandles.length}`,
      `Triangles: ${Math.round(tri).toLocaleString()}`,
    ].join('\n');
  }

  // ------------------------------------------------------------------- posing
  private randomize(): void {
    for (const h of this.morphHandles) {
      if (h.spec.category.startsWith('expression-') || h.spec.category.startsWith('speech-')) continue;
      const minimum = parseFloat(h.slider.min);
      const value = minimum + Math.random() * (1 - minimum);
      h.slider.value = String(value);
      this.applyMorphControl(h, value);
    }
    this.showStatus('Morphs randomized.');
  }

  private reset(): void {
    for (const h of this.morphHandles) {
      h.slider.value = '0';
      this.applyMorphControl(h, 0);
    }
    this.expressionPresetSelect.selectedIndex = 0;
    this.speechPresetSelect.selectedIndex = 0;
    const ethnicity = document.getElementById('ethnicity-select') as HTMLSelectElement;
    ethnicity.value = 'caucasian';
    this.setEthnicity('caucasian');
    this.showStatus('Morphs reset.');
  }

  // ------------------------------------------------------------- skeleton view
  private setSkeletonVisible(visible: boolean): void {
    if (this.skeletonHelper) {
      this.scene.remove(this.skeletonHelper);
      this.skeletonHelper.dispose();
      this.skeletonHelper = undefined;
    }
    if (visible && this.root) {
      this.skeletonHelper = new THREE.SkeletonHelper(this.root);
      this.scene.add(this.skeletonHelper);
    }
  }

  private toggleSkeleton(visible: boolean): void {
    this.setSkeletonVisible(visible);
  }

  private fitCameraTo(obj: THREE.Object3D): void {
    const box = new THREE.Box3().setFromObject(obj);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3()).length();
    this.controls.target.copy(center);
    const dist = size * 1.4;
    this.camera.position.copy(center).add(new THREE.Vector3(dist * 0.7, dist * 0.5, dist));
    this.camera.near = dist / 100;
    this.camera.far = dist * 100;
    this.camera.updateProjectionMatrix();
  }

  private showStatus(msg: string): void {
    this.statusEl.textContent = msg;
  }

  // ------------------------------------------------------------------- loop
  private animate(): void {
    this.renderer.setAnimationLoop((time) => {
      this.controls.update();
      if (this.zonePointerPending && time - this.lastZoneSample >= 50) {
        this.zonePointerPending = false;
        this.lastZoneSample = time;
        this.updateZoneHover();
      }
      this.renderer.render(this.scene, this.camera);
    });
  }
}

const editor = new CharacterEditor();
void editor.loadDefault();
