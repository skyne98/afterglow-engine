const { PNG } = require('pngjs');
const fs = require('fs');
const a = PNG.sync.read(fs.readFileSync('/tmp/e2e_run1.png'));
const b = PNG.sync.read(fs.readFileSync('/tmp/e2e_run2.png'));
console.log('run1', a.width+'x'+a.height, 'run2', b.width+'x'+b.height);
if (a.width!==b.width||a.height!==b.height){console.log('SIZE MISMATCH');process.exit(1);}
let diff=0; const n=a.width*a.height;
for(let i=0;i<a.data.length;i+=4){
  if(Math.abs(a.data[i]-b.data[i])>2||Math.abs(a.data[i+1]-b.data[i+1])>2||Math.abs(a.data[i+2]-b.data[i+2])>2) diff++;
}
const pct=(100*diff/n).toFixed(4);
console.log(`different pixels: ${diff}/${n} = ${pct}%`);
console.log(pct==='0.0000' ? 'DETERMINISM PASS (identical across runs)' : 'determinism: see diff');
// also report a sample pixel
const c=(x,y)=>{const i=(y*a.width+x)*4;return [a.data[i],a.data[i+1],a.data[i+2]];};
console.log('corner(0,0):',c(0,0).join(','),'center(200,125):',c(200,125).join(','));
