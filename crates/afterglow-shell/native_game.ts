import * as THREE from './three/three.webgpu.js';

const canvas = globalThis.engineCanvas;

const hudStyle = document.createElement('style');
hudStyle.textContent = `
  #native-hud { position:fixed; inset:0; z-index:10; pointer-events:none;
    color:#f5f8ff; font:14px sans-serif; }
  #native-hud .panel { position:absolute; top:18px; left:18px; min-width:220px;
    padding:14px 16px; border:1px solid rgba(145,180,255,.48); border-radius:10px;
    background:rgba(5,10,22,.78); box-shadow:0 8px 28px rgba(0,0,0,.38); }
  #native-hud h1 { margin:0 0 5px; font-size:16px; letter-spacing:.04em; }
  #native-hud p { margin:3px 0 11px; color:#b9c7e8; }
  #native-hud button { pointer-events:auto; appearance:none; padding:7px 11px;
    border:1px solid #75a3ff; border-radius:6px; color:white; background:#2459b8; }
  #native-hud button:hover { background:#3471dc; }
  #native-hud .status { position:absolute; top:18px; right:18px; padding:7px 10px;
    border-radius:999px; background:rgba(5,10,22,.72); color:#8ff0b5; }
`;
document.head.appendChild(hudStyle);
const hud = document.createElement('div');
hud.id = 'native-hud';
hud.innerHTML = `<section class="panel"><h1>afterglow-shell</h1>
  <p>Shared-device WebGPU + Blitz HUD</p><button type="button">Change material</button></section>
  <div class="status">● renderer ready</div>`;
document.body.appendChild(hud);

const renderer = new THREE.WebGPURenderer({ canvas, antialias: true });
renderer.setPixelRatio(1);
renderer.setSize(canvas.width, canvas.height, false);
renderer.outputColorSpace = THREE.SRGBColorSpace;

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x070b14);

const camera = new THREE.PerspectiveCamera(55, canvas.width / canvas.height, 0.1, 200);
camera.position.set(0, 1.4, 5);
camera.lookAt(0, 0, 0);

const geometry = new THREE.TorusKnotGeometry(1, 0.34, 192, 32);
const material = new THREE.MeshStandardMaterial({
  color: 0x4b8cff,
  metalness: 0.65,
  roughness: 0.22,
});
const mesh = new THREE.Mesh(geometry, material);
scene.add(mesh);
let alternateMaterial = false;
hud.querySelector('button').addEventListener('click', () => {
  alternateMaterial = !alternateMaterial;
  material.color.setHex(alternateMaterial ? 0xff7a45 : 0x4b8cff);
  hud.querySelector('button').textContent = alternateMaterial ? 'Restore material' : 'Change material';
});
scene.add(new THREE.HemisphereLight(0xbfd8ff, 0x18100a, 1.8));
const key = new THREE.DirectionalLight(0xffffff, 4);
key.position.set(3, 4, 5);
scene.add(key);

let pointerX = 0;
let pointerY = 0;
canvas.addEventListener('pointermove', (event) => {
  pointerX = (Number(event.clientX) / Math.max(1, innerWidth) - 0.5) * 2;
  pointerY = (Number(event.clientY) / Math.max(1, innerHeight) - 0.5) * 2;
});

await renderer.init();
const started = performance.now();
globalThis.renderEngineFrame = () => {
  const time = (performance.now() - started) / 1000;
  mesh.rotation.x = time * 0.32 + pointerY * 0.25;
  mesh.rotation.y = time * 0.55 + pointerX * 0.4;
  renderer.render(scene, camera);
};

globalThis.resizeEngineGame = (width, height, scaleFactor = devicePixelRatio) => {
  width = Math.max(1, Number(width));
  height = Math.max(1, Number(height));
  resizeEngineCanvas(width, height, scaleFactor);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
  renderer.setSize(width, height, false);
};

renderEngineFrame();
globalThis.__syncBrowserDocument(false);
Deno.core.ops.op_present_surface();
Deno.core.ops.op_engine_ready();

const animate = () => {
  renderEngineFrame();
  requestAnimationFrame(animate);
};
requestAnimationFrame(animate);
