// End-to-end op-bridge probe: call the natively-spawned Physics worker (id 0,
// method 0 = `step`) through `op_afterglow_rpc_call` from JS in a real shell run.
// step(vec![0,1,2], 0.5) -> vec![0.5,1.5,2.5].

const f32le = (x) => {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setFloat32(0, x, true);
  return [...b];
};
const varint = (n) => {
  const out = [];
  do { let x = n & 0x7f; n = Math.floor(n / 128); if (n) x |= 0x80; out.push(x); } while (n);
  return out;
};

// postcard(Vec<f32>, f32) = varint(count) + count*f32 + f32
const args = new Uint8Array([
  ...varint(3), ...f32le(0.0), ...f32le(1.0), ...f32le(2.0), ...f32le(0.5),
]);

const resp = Deno.core.ops.op_afterglow_rpc_call(0, 0, args);

// Decode postcard Vec<f32>: varint(count) + count*f32
const dv = new DataView(resp.buffer, resp.byteOffset, resp.byteLength);
let off = 0;
let count = 0;
for (let shift = 0; ; shift += 7) {
  const b = dv.getUint8(off++);
  count += (b & 0x7f) << shift;
  if (!(b & 0x80)) break;
}
const out = [];
for (let i = 0; i < count; i++) out.push(dv.getFloat32(off + i * 4, true));

const ok = JSON.stringify(out) === '[0.5,1.5,2.5]';
console.log(ok ? 'OP_BRIDGE_OK ' + JSON.stringify(out) : 'OP_BRIDGE_FAIL ' + JSON.stringify(out));
