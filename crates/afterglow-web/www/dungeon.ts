import { TextureClient } from './texture.client.ts';
import { Rpc } from './rpc.ts';
import { createFetchRangeLoader, createPageDataProvider, getVirtualTextureDimensions, parseBigHeader } from './engine/big-parser.ts';
import { PersistentBlobCache, persistentCacheNamespace } from './engine/persistent-blob-cache.ts';
import { createWebGPUOnlyRenderer, showWebGPUFailure } from './engine/webgpu-only.ts';
import { moduleRendererFactory } from './engine/renderer-api.ts';
import { assertHeightTextureGpuFormat, loadHeightTextureR16 } from './engine/height-texture.ts';
import { RendererSeal, warmRendererVariants } from './engine/renderer-seal.ts';
import { RelativePointerInput } from './engine/relative-pointer.ts';
import * as THREE from 'three/webgpu';
import * as VT from './engine/virtual-texturing-api.ts';
import { assertPomGeneratedWgsl } from './engine/surface-detail-api.ts';
const VT_QUALITY_BIAS=0, FEEDBACK_INTERVAL=8;
const POM_MIN_LAYERS=8,POM_MAX_LAYERS=32,POM_HEIGHT_SCALE=.05,POM_MAX_OFFSET_RATIO=2,POM_MAX_DISTANCE=0,POM_SHADOW_STEPS=8,POM_SHADOW_BIAS=.01,POM_SHADOW_STRENGTH=.82;
let pomEnabled=true;
const scene=new THREE.Scene();scene.background=new THREE.Color(0x101318);scene.fog=new THREE.Fog(0x101318,7,28);
const camera=new THREE.PerspectiveCamera(70,innerWidth/innerHeight,.05,60);camera.rotation.order='YXZ';
// Four VT-backed PBR channels are substantially more expensive under 4x MSAA.
// The engine demo renders one sample per pixel; production AA should be a
// temporal/post-process pass rather than multiplying all VT lookups.
const renderer=await createWebGPUOnlyRenderer({antialias:false,trackTimestamp:false},moduleRendererFactory).catch(error=>{showWebGPUFailure(error);throw error});renderer.setSize(innerWidth,innerHeight);renderer.setPixelRatio(devicePixelRatio);document.body.append(renderer.domElement);
const rendererSeal=new RendererSeal(renderer.backend),rendererSealStats={renderPipelines:0,computePipelines:0,renderPipelineViolations:0,computePipelineViolations:0};function pipelineTelemetry(){rendererSealStats.renderPipelines=rendererSeal.renderPipelines;rendererSealStats.computePipelines=rendererSeal.computePipelines;rendererSealStats.renderPipelineViolations=rendererSeal.renderPipelineViolations;rendererSealStats.computePipelineViolations=rendererSeal.computePipelineViolations;return rendererSealStats}const errors=[];renderer.backend.device.addEventListener('uncapturederror',e=>errors.push(String(e.error?.message??e.error)));addEventListener('error',e=>errors.push(String(e.error?.stack??e.message)));addEventListener('unhandledrejection',e=>errors.push(String(e.reason?.stack??e.reason)));
scene.add(new THREE.HemisphereLight(0xb9c8e8,0x241b15,1.6));const lamp=new THREE.PointLight(0xffc985,30,18,2);lamp.position.set(0,3.2,0);scene.add(lamp);
const floor=new THREE.Mesh(new THREE.PlaneGeometry(16,16),new THREE.MeshStandardMaterial({color:0x292722,roughness:1}));floor.rotation.x=-Math.PI/2;scene.add(floor);
const ceiling=floor.clone();ceiling.position.y=4;ceiling.rotation.x=Math.PI/2;ceiling.material=new THREE.MeshStandardMaterial({color:0x18191b,roughness:1});scene.add(ceiling);

const rangeLoader=createFetchRangeLoader();
const TEXTURE_WORKER_COUNT=Math.max(2,Math.min(4,Math.floor((navigator.hardwareConcurrency||4)/2)));
const textureRpcs=await Promise.all(Array.from({length:TEXTURE_WORKER_COUNT},()=>Rpc.create({
  mainWasmUrl:'afterglow_web.wasm',workerJsUrl:'worker.js',workerWasmUrl:'texture.wasm',timeoutMs:10000,
})));
const textureWorkers=textureRpcs.map(rpc=>new TextureClient(rpc));
addEventListener('beforeunload',()=>{for(const rpc of textureRpcs)rpc.terminate()},{once:true});
const prefix=await rangeLoader.read('dungeon.big',0,16);
const dataOffset=Number(new DataView(prefix.buffer,prefix.byteOffset+8,8).getBigUint64(0,true));
const headerBytes=await rangeLoader.read('dungeon.big',0,dataOffset);
const {header}=parseBigHeader(headerBytes);
const rendererDevice=renderer.backend.device as GPUDevice; // @unsafe-cast reason=LegacyRendererDevice issue=DME-030 expires=2026-10-01
const format=rendererDevice.features.has('texture-compression-bc')?0:rendererDevice.features.has('texture-compression-astc')?1:VT.FORMAT_RGBA;
const sourceIdentity=await rangeLoader.identity('dungeon.big');
const adapterInfo=renderer.afterglowAdapterInfo??{};
let persistentCache=null;
if(sourceIdentity.etag||sourceIdentity.lastModified){
  try{
    const namespace=await persistentCacheNamespace([
      'afterglow-cache-v1','dungeon.big',String(sourceIdentity.size),
      sourceIdentity.etag??'',sourceIdentity.lastModified??'',
      String(format),'basisu-transcoder-v1','slot-136-border-4',
      adapterInfo.vendor??'',adapterInfo.architecture??'',adapterInfo.device??'',adapterInfo.description??'',
    ]);
    persistentCache=await PersistentBlobCache.open({
      namespace,maxBytes:1024*1024*1024,maxEntries:65536,writeQueueCapacity:64,
    });
  }catch(error){console.warn('[cache] persistent blob cache unavailable:',error)}
}else console.warn('[cache] source has no ETag/Last-Modified; persistent VT cache disabled');
const containerLoader={
  load:path=>rangeLoader.load(path),size:path=>rangeLoader.size(path),
  read:(_path,offset,len)=>rangeLoader.read('dungeon.big',offset,len),
};
const pageProvider=createPageDataProvider(containerLoader,header,textureWorkers,format,persistentCache??undefined);
const loader={read:(path,offset,len)=>rangeLoader.read(path,offset,len),poll(){}};
// One central bootstrap resource owns bounded admission. It receives rAF
// intervals below, probes upward only after stable backlog, and rolls a bad
// promoted cap back to the independently validated two-page baseline.
const vtTuning=new VT.VirtualTextureTuning();
const store=new VT.VirtualTextureStore(loader,pageProvider,format,rendererDevice,vtTuning);
const feedbackPass=new VT.VirtualTextureFeedbackPass(.125);
const pomBinding=new VT.VirtualPomSceneBinding({
  camera,store,feedbackPixelScale:feedbackPass.pixelScale,capacity:12,
  material:{minLayers:POM_MIN_LAYERS,maxLayers:POM_MAX_LAYERS,heightScale:POM_HEIGHT_SCALE,maxOffsetRatio:POM_MAX_OFFSET_RATIO,maxDistance:POM_MAX_DISTANCE,shadowSteps:POM_SHADOW_STEPS,shadowBias:POM_SHADOW_BIAS,shadowStrength:POM_SHADOW_STRENGTH,qualityBias:VT_QUALITY_BIAS,addressMode:1,side:THREE.DoubleSide},
});
const feedbackScene=pomBinding.feedbackScene;
const materialNames=['Rock064','Ground103','PavingStones150'];
// Offline-decoded resident R16 source maps are expanded losslessly to
// filterable single-channel R32F. Every 16-bit source level remains distinct
// throughout the non-uniform march; browser image decoding is bypassed.
const heightThree=THREE as unknown as Parameters<typeof loadHeightTextureR16>[0]; // @unsafe-cast reason=HeightThreeConstructorVariance issue=DME-030 expires=2026-10-01
const heightTextures=await Promise.all(materialNames.map(name=>loadHeightTextureR16(heightThree,rendererDevice,`dungeon-height/${name}_Height.r16`)));
const materialSets=materialNames.map(name=>{const paths={albedo:`${name}_Color.png`,normal:`${name}_NormalGL.png`,masks:`${name}_Masks.png`};const dimensions=getVirtualTextureDimensions(header,paths.albedo);return store.loadMaterialSet(paths,{...dimensions,mipTail:true})});
const segments:Array<readonly [number,number,number,number]>=[
  [-8,-8,8,-8], [8,-8,8,8], [8,8,-8,8], [-8,8,-8,-8],
  [-3,-8,-3,1], [-3,1,2,1], [2,1,2,8],
  [3,-8,3,-1], [-2,-1,3,-1], [-2,-1,-2,5], [-2,5,4,5], [4,5,4,8],
];
const walls=[];
for(let i=0;i<segments.length;i++){
  const segment=segments[i],set=materialSets[i%materialSets.length],heightTexture=heightTextures[i%heightTextures.length];
  if(!segment||!set||!heightTexture)throw new Error('dungeon wall material layout is incomplete');
  const [x1,z1,x2,z2]=segment,entry=set.albedo,path=entry.path;
  const dx=x2-x1,dz=z2-z1,len=Math.hypot(dx,dz),geometry=new THREE.PlaneGeometry(len,4,1,1);
  // `parallaxDirection` needs the same explicit local +X tangent used by the
  // validated prototype; derivative TBN fallback is sufficient for normalMap
  // but does not provide a stable tangent-space view ray for POM.
  geometry.setAttribute('tangent',new THREE.BufferAttribute(new Float32Array([1,0,0,1,1,0,0,1,1,0,0,1,1,0,0,1]),4));
  // Preserve brick proportions: one square virtual-texture repeat per 4 m of
  // wall instead of stretching a square texture across arbitrarily long runs.
  const wallUv=geometry.getAttribute('uv');
  for(let u=0;u<wallUv.count;u++)wallUv.setX(u,wallUv.getX(u)*len/4);
  const placeholder=new THREE.MeshStandardMaterial();
  const mesh=new THREE.Mesh(geometry,placeholder);
  mesh.position.set((x1+x2)/2,2,(z1+z2)/2);mesh.rotation.y=Math.atan2(-dz,dx);scene.add(mesh);
  const pomHeight=heightTexture as unknown as THREE.Texture; // @unsafe-cast reason=HeightTextureStructuralType issue=DME-030 expires=2026-10-01
  pomBinding.add(mesh,set,pomHeight);placeholder.dispose();
  walls.push({path,entry,x1,z1,x2,z2,len,mesh});
}
pomBinding.seal();

const PLAYER_RADIUS=.28, pose={x:-5.5,z:-5.5,yaw:0,pitch:0}, keys=new Set();let programmatic=false,diagnosticAtlas=false,feedbackEnabled=true,last=performance.now(),smoothedDt=1/60,frame=0,lastResult={loaded:0,evicted:0,totalRequests:0,lodBias:0};
const runtimeTiming={vtCpuUs:0,renderSubmitUs:0,feedbackSubmitUs:0,frameCpuUs:0,gpuMainMs:0,gpuFeedbackMs:0,gpuTotalMs:0,gpuTimestampSupported:Boolean(renderer.backend.hasTimestamp)};let resolvingGpuTimestamps=false;
async function resolveGpuTimings(){
  if(!runtimeTiming.gpuTimestampSupported||resolvingGpuTimestamps)return runtimeTiming;
  resolvingGpuTimestamps=true;
  try{
    runtimeTiming.gpuTotalMs=await renderer.resolveTimestampsAsync('render');
    const contexts=renderer._renderContexts,pool=renderer.backend.timestampQueryPool?.render,timestamps=pool?.timestamps;
    if(contexts&&timestamps){
      const mainContext=contexts.get(null).id,feedbackContext=contexts.get(feedbackPass.target).id;let mainFrame=-1,feedbackFrame=-1;
      for(const [uid,duration] of timestamps){const parts=uid.split(':'),context=Number(parts[2]),id=Number(parts[3]?.slice(1));if(context===mainContext&&id>mainFrame){mainFrame=id;runtimeTiming.gpuMainMs=duration}else if(context===feedbackContext&&id>feedbackFrame){feedbackFrame=id;runtimeTiming.gpuFeedbackMs=duration}}
      timestamps.clear();const frames=pool.getTimestampFrames?.();if(frames)frames.length=0;
    }
  }finally{resolvingGpuTimestamps=false}
  return runtimeTiming;
}
function setGpuTimingEnabled(enabled){const active=Boolean(enabled)&&runtimeTiming.gpuTimestampSupported;renderer.backend.trackTimestamp=active;for(const pool of Object.values(renderer.backend.timestampQueryPool??{}))if(pool)pool.trackTimestamp=active}
function pointSegmentDistance(x,z,s){const dx=s.x2-s.x1,dz=s.z2-s.z1,l2=dx*dx+dz*dz,t=Math.max(0,Math.min(1,((x-s.x1)*dx+(z-s.z1)*dz)/l2)),px=s.x1+t*dx,pz=s.z1+t*dz;return Math.hypot(x-px,z-pz)}
function valid(x,z){return x>-7.7&&x<7.7&&z>-7.7&&z<7.7&&walls.slice(4).every(s=>pointSegmentDistance(x,z,s)>PLAYER_RADIUS)}
function setPose(x,z,yaw=pose.yaw,pitch=pose.pitch){if(valid(x,z)){pose.x=x;pose.z=z}pose.yaw=yaw;pose.pitch=Math.max(-1.45,Math.min(1.45,pitch))}
function move(forward,strafe){const sin=Math.sin(pose.yaw),cos=Math.cos(pose.yaw),dx=(-sin*forward+cos*strafe),dz=(-cos*forward-sin*strafe);if(valid(pose.x+dx,pose.z))pose.x+=dx;if(valid(pose.x,pose.z+dz))pose.z+=dz}
function update(dt){
  let f=(keys.has('w')?1:0)-(keys.has('s')?1:0),s=(keys.has('d')?1:0)-(keys.has('a')?1:0),sprint=keys.has('shift')?5.5:2.8;if(f||s){const n=Math.hypot(f,s);move(f/n*sprint*dt,s/n*sprint*dt)}
  camera.position.set(pose.x,1.7,pose.z);camera.rotation.set(pose.pitch,pose.yaw,0);camera.updateMatrixWorld();lamp.position.set(pose.x,3.1,pose.z);
  const stageStart=performance.now();const feedback=feedbackPass.consume();if(feedback&&!diagnosticAtlas)lastResult=store.processFeedback(feedback);store.poll();runtimeTiming.vtCpuUs=(performance.now()-stageStart)*1000;
}
feedbackPass.resize(renderer.domElement.width,renderer.domElement.height);
// Inspect generated WGSL during bootstrap: Three's lazy normal flow can silently
// reorder a normal-map-dependent POM ray before UV initialization.
let pomShaderContracts=0,pomFeedbackContracts=0;const gpuDevice=rendererDevice,createShaderModule=gpuDevice.createShaderModule.bind(gpuDevice);gpuDevice.createShaderModule=descriptor=>{if(descriptor.code.includes('fn pomMarchUV')){if(descriptor.code.includes('fn vtSampleFromLevel')){assertPomGeneratedWgsl(descriptor.code);pomShaderContracts++}else if(descriptor.code.includes('fn vtFeedback'))pomFeedbackContracts++;else throw new Error('unknown POM shader variant compiled during warm-up')}return createShaderModule(descriptor)};
// Prewarm render and POM-aware feedback variants before GameplaySealed; runtime
// P toggles only swap fixed references and never compile a pipeline.
pomBinding.setPomEnabled(false);
await warmRendererVariants(renderer,[{scene,camera}]);
const previousTarget=renderer.getRenderTarget();renderer.setRenderTarget(feedbackPass.target);await warmRendererVariants(renderer,[{scene:feedbackScene,camera}]);
pomBinding.setPomEnabled(true);
renderer.setRenderTarget(previousTarget);await warmRendererVariants(renderer,[{scene,camera}]);renderer.setRenderTarget(feedbackPass.target);await warmRendererVariants(renderer,[{scene:feedbackScene,camera}]);renderer.setRenderTarget(previousTarget);gpuDevice.createShaderModule=createShaderModule;if(pomShaderContracts<1||pomFeedbackContracts<1)throw new Error('POM render/feedback shader contracts were not compiled during warm-up');
await new Promise(r=>setTimeout(r,0));renderer.render(scene,camera);for(const height of heightTextures)assertHeightTextureGpuFormat(renderer.backend,height);renderer.setRenderTarget(feedbackPass.target);renderer.render(feedbackScene,camera);renderer.setRenderTarget(previousTarget);store.attachRenderer(renderer as never);rendererSeal.seal(); // @unsafe-cast reason=LegacyRendererAttachment issue=DME-030 expires=2026-10-01
const waiters=[],hud=document.getElementById('hud');let hudVisible=true;
function setPomEnabled(enabled){pomEnabled=Boolean(enabled);pomBinding.setPomEnabled(pomEnabled)}
renderer.setAnimationLoop(now=>{const frameCpuStart=performance.now(),dt=Math.min(.05,(now-last)/1000);last=now;smoothedDt=smoothedDt*.95+dt*.05;store.recordFrameTime(dt*1000);update(dt);const renderStart=performance.now();renderer.render(scene,camera);runtimeTiming.renderSubmitUs=(performance.now()-renderStart)*1000;if(feedbackEnabled&&!diagnosticAtlas&&frame%FEEDBACK_INTERVAL===0){const feedbackStart=performance.now();feedbackPass.submit(renderer,feedbackScene,camera,store);runtimeTiming.feedbackSubmitUs=(performance.now()-feedbackStart)*1000}else runtimeTiming.feedbackSubmitUs=0;runtimeTiming.frameCpuUs=(performance.now()-frameCpuStart)*1000;frame++;for(let i=waiters.length-1;i>=0;i--)if(frame>=waiters[i].target){waiters[i].resolve();waiters.splice(i,1)}if(hudVisible&&frame%15===0){const d=store.getStats(),input=relativePointer.getStatus();hud.innerHTML=`<b>afterglow — Engine Dungeon</b><br>3 × 8K scanned PBR material sets · 12 wall instances<br>Virtual RGBA channels: 1.875 GiB · physical atlas: ${store.atlasWidth}²<br>Position: ${pose.x.toFixed(2)}, ${pose.z.toFixed(2)} · yaw ${(pose.yaw*180/Math.PI).toFixed(0)}° · ${(1/smoothedDt).toFixed(0)} FPS<br>Input: ${input.eventType}${input.unadjustedMovement?' · unadjusted':''}<br>POM: ${pomEnabled?`${POM_MIN_LAYERS}–${POM_MAX_LAYERS} layers · ${POM_SHADOW_STEPS}-step light self-shadow · no radial fade`:'off'}<br>Textures: ${d.textureCount} · resident ${d.atlasSlotsUsed}/${d.atlasSlotsTotal} · pending ${d.pendingPages}<br>GPU feedback pages: ${lastResult.totalRequests} · mips [${feedbackPass.getLatestMips().join(',')}] · quality ${VT_QUALITY_BIAS} · capacity bias ${d.lodBias} · budget ${d.budget} · errors ${errors.length}`}});
addEventListener('resize',()=>{camera.aspect=innerWidth/innerHeight;camera.updateProjectionMatrix();renderer.setSize(innerWidth,innerHeight);feedbackPass.resize(renderer.domElement.width,renderer.domElement.height)});
addEventListener('keydown',e=>{if(programmatic)return;const key=e.key.toLowerCase();keys.add(key);if(key==='r')setPose(-5.5,-5.5,0,0);if(key==='p')setPomEnabled(!pomEnabled);if(e.key==='1')setPose(-5.5,-5.5,0,0);if(e.key==='2')setPose(5.5,-5.5,Math.PI,0);if(e.key==='3')setPose(5.5,6.5,-Math.PI/2,0)});addEventListener('keyup',e=>keys.delete(e.key.toLowerCase()));
const relativePointer=new RelativePointerInput(renderer.domElement,(movementX,movementY)=>{if(!programmatic){pose.yaw-=movementX*.002;pose.pitch=Math.max(-1.45,Math.min(1.45,pose.pitch-movementY*.002))}});
renderer.domElement.addEventListener('click',()=>{if(!programmatic)relativePointer.requestLock()});
const scenarios={forward:()=>setPose(-5.5,-5.5,0,0),reverse:()=>setPose(5.5,-5.5,Math.PI,0),corner:()=>setPose(5.8,6.4,-Math.PI/2,-.2)};
function atlasFeedback(groupCount,startPage=0){
  const feedback=new Map(),albedos=materialSets.map(set=>set.albedo);
  for(let index=0;index<groupCount;index++){
    const entry=albedos[index%albedos.length];if(!entry)throw new Error('atlas feedback material is missing');const local=startPage+Math.floor(index/albedos.length),page=local%(entry.pageGridX*entry.pageGridY);
    feedback.set(index,{path:entry.path,mip:0,x:page%entry.pageGridX,y:Math.floor(page/entry.pageGridX)});
  }
  return feedback;
}
async function waitForAtlas(target,timeout,feedback=null){
  const end=performance.now()+timeout;let steps=0;
  while(performance.now()<end){
    const stats=store.getStats();
    if(stats.atlasSlotsUsed>=target&&!stats.pendingPages&&!stats.scheduledRequests&&!stats.readyUploads)return true;
    if(feedback&&steps%FEEDBACK_INTERVAL===0)store.processFeedback(feedback);
    steps++;await window.__afterglowDungeon.step(1);
  }
  return false;
}
async function runAtlasScenario(name,timeout=120000){
  if(!['cold','half','full','churn'].includes(name))throw new Error(`unknown atlas scenario ${name}`);
  const previousProgrammatic=programmatic;
  programmatic=true;diagnosticAtlas=true;keys.clear();feedbackPass.consume();
  try{
    const initial=store.getStats(),total=initial.atlasSlotsTotal;let target=name==='half'?Math.floor(total/2):name==='cold'?initial.atlasSlotsUsed:Math.floor(total*0.995);
    if(name==='cold'){await waitForAtlas(target,timeout);target=store.getStats().atlasSlotsUsed}else{
      const groups=Math.ceil(Math.max(0,target-initial.atlasSlotsUsed)/3)+32;
      const admission=atlasFeedback(groups,name==='half'?0:1024);
      store.processFeedback(admission);
      await waitForAtlas(target,timeout,admission);
    }
    if(name==='churn'){
      const before=store.getStats().cacheEvictions,groups=Math.ceil(total/3);
      const replacement=atlasFeedback(groups,3072);
      for(let epoch=0;epoch<17;epoch++)store.processFeedback(replacement);
      const end=performance.now()+timeout;let steps=0;
      while(performance.now()<end&&(store.getStats().cacheEvictions===before||store.getStats().pendingPages||store.getStats().scheduledRequests||store.getStats().readyUploads)){
        if(steps%FEEDBACK_INTERVAL===0)store.processFeedback(replacement);
        steps++;await window.__afterglowDungeon.step(1);
      }
    }
    return {name,target,...store.getStats(),timing:{...runtimeTiming},errors:errors.length};
  }finally{diagnosticAtlas=false;programmatic=previousProgrammatic}
}
window.__afterglowDungeon={
  ready:()=>true,telemetry:()=>store.getStats(),timing:()=>runtimeTiming,inputStatus:()=>relativePointer.getStatus(),pomStatus:()=>({enabled:pomEnabled,minLayers:POM_MIN_LAYERS,maxLayers:POM_MAX_LAYERS,heightScale:POM_HEIGHT_SCALE,maxOffsetRatio:POM_MAX_OFFSET_RATIO,maxDistance:POM_MAX_DISTANCE,selfShadowSteps:POM_SHADOW_STEPS,selfShadowStrength:POM_SHADOW_STRENGTH,heightSource:'resident ambientCG displacement',heightFormat:'r32float-from-r16'}),setPomEnabled,setFeedbackEnabled:enabled=>{feedbackEnabled=Boolean(enabled)},pipelineTelemetry,resolveGpuTimings,setGpuTimingEnabled,errorCount:()=>errors.length,runAtlasScenario,snapshot:()=>({pose:{...pose},...store.getDebugSnapshot(),requests:lastResult.totalRequests,feedbackMips:[...feedbackPass.getLatestMips()],errors:[...errors]}),
  setProgrammatic:enabled=>{programmatic=Boolean(enabled);keys.clear();if(programmatic&&document.pointerLockElement)document.exitPointerLock()},setHudVisible:visible=>{hudVisible=Boolean(visible);hud.style.display=hudVisible?'':'none'},
  setPose,getPose:()=>({...pose}),move,look:(yaw,pitch)=>setPose(pose.x,pose.z,pose.yaw+yaw,pose.pitch+pitch),
  step:n=>new Promise(resolve=>waiters.push({target:frame+Math.max(1,n|0),resolve})),
  waitForIdle:async(timeout=5000)=>{const end=performance.now()+timeout;while((store.getStats().pendingPages||store.getStats().scheduledRequests||store.getStats().readyUploads)&&performance.now()<end)await window.__afterglowDungeon.step(1);return store.getStats().pendingPages===0&&store.getStats().scheduledRequests===0&&store.getStats().readyUploads===0},
  runScenario:async name=>{if(!scenarios[name])throw new Error(`unknown scenario ${name}`);programmatic=true;keys.clear();scenarios[name]();await window.__afterglowDungeon.step(120);await window.__afterglowDungeon.waitForIdle(15000);await window.__afterglowDungeon.step(16);await window.__afterglowDungeon.waitForIdle(15000);return window.__afterglowDungeon.snapshot()},
};
