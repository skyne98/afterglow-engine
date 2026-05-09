import { requireTools } from "../utils/detect";
import { flakeRoot, runInDevShell, inNixShell } from "../utils/nix";

const FLAGS = "--profile wasm-release --target wasm32-unknown-unknown -p agx";

async function hasWasmBindgen(): Promise<boolean> {
  const r = Bun.spawnSync(["which", "wasm-bindgen"]);
  return r.exitCode === 0;
}

export async function buildWasm(): Promise<void> {
  await requireTools(["cargo"], "Run `nix develop` or add to NixOS packages");

  const inShell = await inNixShell();
  const hasWbg = await hasWasmBindgen();

  if (!hasWbg && !inShell) {
    // Enter nix shell which provides wasm-bindgen
    const cmd = [
      `cargo build ${FLAGS}`,
      `wasm-bindgen --out-dir wasm --web target/wasm32-unknown-unknown/wasm-release/agx.wasm`,
    ].join(" && ");

    const code = await runInDevShell(cmd);
    if (code !== 0) process.exit(code);
    return;
  }

  if (!hasWbg) {
    console.error("\n  Missing tool: wasm-bindgen");
    console.error("  Ensure your flake/nix-shell provides wasm-bindgen");
    process.exit(1);
  }

  await $`cargo build ${FLAGS}`;
  await $`wasm-bindgen --out-dir wasm --web target/wasm32-unknown-unknown/wasm-release/agx.wasm`;
}
