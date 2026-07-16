import { TextureClient } from './texture.client.ts';
import { Rpc } from './rpc.ts';
import { createFetchRangeLoader, createPageDataProvider, getVirtualTextureDimensions, parseBigHeader } from './engine/big-parser.ts';
import { PersistentBlobCache, persistentCacheNamespace } from './engine/persistent-blob-cache.ts';
import { createWebGPUOnlyRenderer, showWebGPUFailure } from './engine/webgpu-only.ts';
import { RendererSeal, warmRendererVariants } from './engine/renderer-seal.ts';
import { RelativePointerInput } from './engine/relative-pointer.ts';
const THREE=window.THREE, VT=window.AfterglowVT;
if(!VT) throw new Error('AfterglowVT engine bundle is unavailable');
const {wgslFn,Fn,texture,sampler,uv,uniform,float,uint}=THREE;
const VT_LOD_BIAS=-1.5, FEEDBACK_INTERVAL=4;
const scene=new THREE.Scene();scene.background=new THREE.Color(0x101318);scene.fog=new THREE.Fog(0x101318,7,28);
const camera=new THREE.PerspectiveCamera(70,innerWidth/innerHeight,.05,60);camera.rotation.order='YXZ';
// Four VT-backed PBR channels are substantially more expensive under 4x MSAA.
// The engine demo renders one sample per pixel; production AA should be a
// temporal/post-process pass rather than multiplying all VT lookups.
const renderer=await createWebGPUOnlyRenderer({antialias:false,trackTimestamp:true}).catch(error=>{showWebGPUFailure(error);throw error});renderer.setSize(innerWidth,innerHeight);renderer.setPixelRatio(devicePixelRatio);document.body.append(renderer.domElement);
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
const prefix=await rangeLoader.read('vt-dungeon.big',0,16);
const dataOffset=Number(new DataView(prefix.buffer,prefix.byteOffset+8,8).getBigUint64(0,true));
const headerBytes=await rangeLoader.read('vt-dungeon.big',0,dataOffset);
const {header}=parseBigHeader(headerBytes);
const format=renderer.backend.device.features.has('texture-compression-bc')?0:renderer.backend.device.features.has('texture-compression-astc')?1:VT.FORMAT_RGBA;
const sourceIdentity=await rangeLoader.identity('vt-dungeon.big');
const adapterInfo=renderer.afterglowAdapterInfo??{};
let persistentCache=null;
if(sourceIdentity.etag||sourceIdentity.lastModified){
  try{
    const namespace=await persistentCacheNamespace([
      'afterglow-cache-v1','vt-dungeon.big',String(sourceIdentity.size),
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
  read:(_path,offset,len)=>rangeLoader.read('vt-dungeon.big',offset,len),
};
const pageProvider=createPageDataProvider(containerLoader,header,textureWorkers,format,persistentCache??undefined);
const loader={read:(path,offset,len)=>rangeLoader.read(path,offset,len),poll(){}};
// One central bootstrap resource owns bounded admission. It receives rAF
// intervals below, probes upward only after stable backlog, and rolls a bad
// promoted cap back to the independently validated two-page baseline.
const vtTuning=new VT.VirtualTextureTuning();
const store=new VT.VirtualTextureStore(loader,pageProvider,format,renderer.backend.device,vtTuning);
const vtSampleLevel=wgslFn(VT.VT_SAMPLE_LEVEL_WGSL),vtResolveMaterialMip4=wgslFn(VT.VT_RESOLVE_MATERIAL_MIP4_WGSL),vtFeedback=wgslFn(VT.VT_FEEDBACK_WGSL),atlasNode=texture(store.atlasTexture),atlasSampler=sampler(atlasNode);
const feedbackScene=new THREE.Scene(),feedbackPass=new VT.VirtualTextureFeedbackPass(.125);
const materialNames=['Rock064','Ground103','PavingStones150'];
const materialSets=materialNames.map(name=>{const paths={albedo:`${name}_Color.png`,normal:`${name}_NormalGL.png`,masks:`${name}_Masks.png`};const dimensions=getVirtualTextureDimensions(header,paths.albedo);return store.loadMaterialSet(paths,{...dimensions,mipTail:true})});
const segments=[
  [-8,-8,8,-8], [8,-8,8,8], [8,8,-8,8], [-8,8,-8,-8],
  [-3,-8,-3,1], [-3,1,2,1], [2,1,2,8],
  [3,-8,3,-1], [-2,-1,3,-1], [-2,-1,-2,5], [-2,5,4,5], [4,5,4,8],
];
const walls=[];
function feedbackMaterial(entry){
  const material=new THREE.MeshBasicNodeMaterial({side:THREE.DoubleSide});
  material.fragmentNode=Fn(()=>vtFeedback({uv:uv().fract(),virtualSize:uniform(new THREE.Vector2(entry.width,entry.height)),pageGrid:uniform(new THREE.Vector2(entry.pageGridX,entry.pageGridY)),maxMip:float(entry.maxMip),lodBias:float(VT_LOD_BIAS),textureId:uint(entry.textureId)}))();
  return material;
}
function sampleEntryAtMip(entry,resolvedMip){const pageTable=texture(entry.pageTableTexture);return vtSampleLevel({pageTable,atlas:atlasNode,atlasSampler,uv:uv(),virtualSize:uniform(new THREE.Vector2(entry.width,entry.height)),pageGrid:uniform(new THREE.Vector2(entry.pageGridX,entry.pageGridY)),pageSize:float(VT.PAGE_SIZE),pageBorder:float(VT.PAGE_BORDER),atlasSize:uniform(new THREE.Vector2(store.atlasWidth,store.atlasHeight)),maxMip:float(entry.maxMip),resolvedMip,addressMode:uint(1)})}
function wallMaterial(set){
  if(!set.normal||!set.masks)throw new Error('dungeon PBR material set requires albedo, normal, and packed masks');
  const material=new THREE.MeshStandardNodeMaterial({metalness:0,side:THREE.DoubleSide});
  const resolvedMip=Fn(()=>vtResolveMaterialMip4({pageTable0:texture(set.albedo.pageTableTexture),pageTable1:texture(set.normal.pageTableTexture),pageTable2:texture(set.masks.pageTableTexture),pageTable3:texture(set.masks.pageTableTexture),uv:uv(),virtualSize:uniform(new THREE.Vector2(set.albedo.width,set.albedo.height)),pageGrid:uniform(new THREE.Vector2(set.albedo.pageGridX,set.albedo.pageGridY)),pageSize:float(VT.PAGE_SIZE),maxMip:float(set.albedo.maxMip),textureMaxMip:float(set.albedo.textureMaxMip),addressMode:uint(1)}))().toVar();
  // Resolve one level across all PBR page tables, then sample every channel at
  // that level. Independent residency can no longer mix material mip levels.
  material.colorNode=Fn(()=>{const color=sampleEntryAtMip(set.albedo,resolvedMip);return THREE.vec4(THREE.sRGBTransferEOTF(color.rgb),color.a)})();
  const masks=Fn(()=>sampleEntryAtMip(set.masks,resolvedMip))().toVar();
  material.normalNode=Fn(()=>THREE.normalMap(sampleEntryAtMip(set.normal,resolvedMip).xyz))();
  material.roughnessNode=Fn(()=>masks.r)();
  material.aoNode=Fn(()=>masks.g)();
  return material;
}
for(let i=0;i<segments.length;i++){
  const [x1,z1,x2,z2]=segments[i],set=materialSets[i%materialSets.length],entry=set.albedo,path=entry.path;
  const dx=x2-x1,dz=z2-z1,len=Math.hypot(dx,dz),geometry=new THREE.PlaneGeometry(len,4,1,1);
  // Preserve brick proportions: one square virtual-texture repeat per 4 m of
  // wall instead of stretching a square texture across arbitrarily long runs.
  for(let u=0;u<geometry.attributes.uv.count;u++)geometry.attributes.uv.setX(u,geometry.attributes.uv.getX(u)*len/4);
  const mesh=new THREE.Mesh(geometry,wallMaterial(set));
  mesh.position.set((x1+x2)/2,2,(z1+z2)/2);mesh.rotation.y=Math.atan2(-dz,dx);scene.add(mesh);
  const feedbackMesh=new THREE.Mesh(geometry,feedbackMaterial(entry));feedbackMesh.position.copy(mesh.position);feedbackMesh.rotation.copy(mesh.rotation);feedbackScene.add(feedbackMesh);
  walls.push({path,entry,x1,z1,x2,z2,len,mesh,feedbackMesh});
}

const PLAYER_RADIUS=.28, pose={x:-5.5,z:-5.5,yaw:0,pitch:0}, keys=new Set();let programmatic=false,diagnosticAtlas=false,last=performance.now(),smoothedDt=1/60,frame=0,lastResult={loaded:0,evicted:0,totalRequests:0,lodBias:0};
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
await warmRendererVariants(renderer,[{scene,camera}]);const previousTarget=renderer.getRenderTarget();renderer.setRenderTarget(feedbackPass.target);await warmRendererVariants(renderer,[{scene:feedbackScene,camera}]);renderer.setRenderTarget(previousTarget);
await new Promise(r=>setTimeout(r,0));renderer.render(scene,camera);renderer.setRenderTarget(feedbackPass.target);renderer.render(feedbackScene,camera);renderer.setRenderTarget(previousTarget);store.attachRenderer(renderer);rendererSeal.seal();
const waiters=[],hud=document.getElementById('hud');let hudVisible=true;
renderer.setAnimationLoop(now=>{const frameCpuStart=performance.now(),dt=Math.min(.05,(now-last)/1000);last=now;smoothedDt=smoothedDt*.95+dt*.05;store.recordFrameTime(dt*1000);update(dt);const renderStart=performance.now();renderer.render(scene,camera);runtimeTiming.renderSubmitUs=(performance.now()-renderStart)*1000;if(!diagnosticAtlas&&frame%FEEDBACK_INTERVAL===0){const feedbackStart=performance.now();feedbackPass.submit(renderer,feedbackScene,camera,store);runtimeTiming.feedbackSubmitUs=(performance.now()-feedbackStart)*1000}else runtimeTiming.feedbackSubmitUs=0;runtimeTiming.frameCpuUs=(performance.now()-frameCpuStart)*1000;frame++;for(let i=waiters.length-1;i>=0;i--)if(frame>=waiters[i].target){waiters[i].resolve();waiters.splice(i,1)}if(hudVisible&&frame%15===0){const d=store.getStats(),input=relativePointer.getStatus();hud.innerHTML=`<b>afterglow — Engine VT Dungeon</b><br>3 × 8K scanned PBR material sets · 12 wall instances<br>Virtual RGBA channels: 1.875 GiB · physical atlas: ${store.atlasWidth}²<br>Position: ${pose.x.toFixed(2)}, ${pose.z.toFixed(2)} · yaw ${(pose.yaw*180/Math.PI).toFixed(0)}° · ${(1/smoothedDt).toFixed(0)} FPS<br>Input: ${input.eventType}${input.unadjustedMovement?' · unadjusted':''}<br>Textures: ${d.textureCount} · resident ${d.atlasSlotsUsed}/${d.atlasSlotsTotal} · pending ${d.pendingPages}<br>GPU feedback pages: ${lastResult.totalRequests} · mips [${feedbackPass.getLatestMips().join(',')}] · bias ${VT_LOD_BIAS} · budget ${d.budget} · errors ${errors.length}`}});
addEventListener('resize',()=>{camera.aspect=innerWidth/innerHeight;camera.updateProjectionMatrix();renderer.setSize(innerWidth,innerHeight);feedbackPass.resize(renderer.domElement.width,renderer.domElement.height)});
addEventListener('keydown',e=>{if(programmatic)return;keys.add(e.key.toLowerCase());if(e.key.toLowerCase()==='r')setPose(-5.5,-5.5,0,0);if(e.key==='1')setPose(-5.5,-5.5,0,0);if(e.key==='2')setPose(5.5,-5.5,Math.PI,0);if(e.key==='3')setPose(5.5,6.5,-Math.PI/2,0)});addEventListener('keyup',e=>keys.delete(e.key.toLowerCase()));
const relativePointer=new RelativePointerInput(renderer.domElement,(movementX,movementY)=>{if(!programmatic){pose.yaw-=movementX*.002;pose.pitch=Math.max(-1.45,Math.min(1.45,pose.pitch-movementY*.002))}});
renderer.domElement.addEventListener('click',()=>{if(!programmatic)relativePointer.requestLock()});
const scenarios={forward:()=>setPose(-5.5,-5.5,0,0),reverse:()=>setPose(5.5,-5.5,Math.PI,0),corner:()=>setPose(5.8,6.4,-Math.PI/2,-.2)};
function atlasFeedback(groupCount,startPage=0){
  const feedback=new Map(),albedos=materialSets.map(set=>set.albedo);
  for(let index=0;index<groupCount;index++){
    const entry=albedos[index%albedos.length],local=startPage+Math.floor(index/albedos.length),page=local%(entry.pageGridX*entry.pageGridY);
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
    steps++;await window.__afterglowVtDungeon.step(1);
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
        steps++;await window.__afterglowVtDungeon.step(1);
      }
    }
    return {name,target,...store.getStats(),timing:{...runtimeTiming},errors:errors.length};
  }finally{diagnosticAtlas=false;programmatic=previousProgrammatic}
}
window.__afterglowVtDungeon={
  ready:()=>true,telemetry:()=>store.getStats(),timing:()=>runtimeTiming,inputStatus:()=>relativePointer.getStatus(),pipelineTelemetry,resolveGpuTimings,setGpuTimingEnabled,errorCount:()=>errors.length,runAtlasScenario,snapshot:()=>({pose:{...pose},...store.getDebugSnapshot(),requests:lastResult.totalRequests,feedbackMips:[...feedbackPass.getLatestMips()],errors:[...errors]}),
  setProgrammatic:enabled=>{programmatic=Boolean(enabled);keys.clear();if(programmatic&&document.pointerLockElement)document.exitPointerLock()},setHudVisible:visible=>{hudVisible=Boolean(visible);hud.style.display=hudVisible?'':'none'},
  setPose,getPose:()=>({...pose}),move,look:(yaw,pitch)=>setPose(pose.x,pose.z,pose.yaw+yaw,pose.pitch+pitch),
  step:n=>new Promise(resolve=>waiters.push({target:frame+Math.max(1,n|0),resolve})),
  waitForIdle:async(timeout=5000)=>{const end=performance.now()+timeout;while((store.getStats().pendingPages||store.getStats().scheduledRequests||store.getStats().readyUploads)&&performance.now()<end)await window.__afterglowVtDungeon.step(1);return store.getStats().pendingPages===0&&store.getStats().scheduledRequests===0&&store.getStats().readyUploads===0},
  runScenario:async name=>{if(!scenarios[name])throw new Error(`unknown scenario ${name}`);programmatic=true;keys.clear();scenarios[name]();await window.__afterglowVtDungeon.step(90);await window.__afterglowVtDungeon.waitForIdle();return window.__afterglowVtDungeon.snapshot()},
};
