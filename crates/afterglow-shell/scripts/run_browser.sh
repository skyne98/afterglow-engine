#!/usr/bin/env bash
# Run a real three.js example through the DOM-emulation + deno_webgpu runtime
# with the real determinism injection, capture the canvas, and pixel-diff
# against the repo's reference screenshot.
#
# Usage: scripts/run_browser.sh [example] [vendor_dir]
set -e
EXAMPLE=${1:-webgpu_clipping}
VENDOR=${2:-/tmp/threejs}

# GPU env (headless wgpu). For rasterizer-matching vs the repo reference (which
# Chrome renders on lavapipe), set VK_ICD_FILENAMES to lvp_icd.x86_64.json.
XLIBS=(libxcb.so.1 libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXrandr.so.1 libXi.so.6 libXrender.so.1 libXext.so.6 libXfixes.so.3 libXau.so.6 libXdmcp.so.6 libXdamage.so.1 libxkbcommon.so libxcb-render.so.0)
LP=""
for lib in "${XLIBS[@]}"; do d=$(find /nix/store -maxdepth 3 -name "$lib" 2>/dev/null | head -1); [ -n "$d" ] && d=$(dirname "$d"); [ -n "$d" ] && LP="$LP:$d"; done
export LD_LIBRARY_PATH="${LP#:}:/run/opengl-driver/lib"
export VK_ICD_FILENAMES=${VK_ICD_FILENAMES:-/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.json}
export VK_DRIVER_FILES="$VK_ICD_FILENAMES"
export NO_ENABLE_TIMELINE_SEMAPHORE=1
export RUSTY_V8_ARCHIVE=${RUSTY_V8_ARCHIVE:-/tmp/v149.a}
unset WAYLAND_DISPLAY DISPLAY

CRATE_ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORKSPACE_ROOT=$(cd "$CRATE_ROOT/../.." && pwd)
cd "$CRATE_ROOT"
[ -f "$VENDOR/build/three.webgpu.js" ] || { echo "vendor three.js first: scripts/vendor_threejs.sh $VENDOR"; exit 1; }

OUT=/tmp/browser_out.png
echo "=== render $EXAMPLE ==="
"$WORKSPACE_ROOT/target/debug/examples/browser_test" "$VENDOR" "$EXAMPLE" "$OUT"

REF="$VENDOR/examples/screenshots/$EXAMPLE.jpg"
if [ -f "$REF" ]; then
  ffmpeg -y -i "$REF" /tmp/ref.png 2>/dev/null
  echo "=== pixel-diff vs repo reference (downscale ours to ref size, 2x2 avg) ==="
  cd cdp_client
  bun add pngjs >/dev/null 2>&1
  bun -e "
    const {PNG}=require('pngjs'); const fs=require('fs');
    const a=PNG.sync.read(fs.readFileSync('$OUT')); const r=PNG.sync.read(fs.readFileSync('/tmp/ref.png'));
    const sx=a.width/r.width|0, sy=a.height/r.height|0;
    let diff=0,sum=0,n=0;
    for(let y=0;y<r.height;y++)for(let x=0;x<r.width;x++){
      let R=0,G=0,B=0,c=0;
      for(let dy=0;dy<sy;dy++)for(let dx=0;dx<sx;dx++){const i=(((y*sy+dy)*a.width)+(x*sx+dx))*4;R+=a.data[i];G+=a.data[i+1];B+=a.data[i+2];c++;}
      R/=c;G/=c;B/=c; const j=(y*r.width+x)*4;
      const d=Math.abs(R-r.data[j])+Math.abs(G-r.data[j+1])+Math.abs(B-r.data[j+2]);
      sum+=d;n++;if(d>30)diff++;
    }
    console.log('ours '+a.width+'x'+a.height+' (downscale '+sx+'x'+sy+') ref '+r.width+'x'+r.height);
    console.log('diff>30: '+diff+'/'+n+' = '+(100*diff/n).toFixed(2)+'%  avg='+(sum/n/3).toFixed(1));
  "
else
  echo "no reference screenshot at $REF; output: $OUT"
fi
