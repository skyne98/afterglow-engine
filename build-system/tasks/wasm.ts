import { runInDevShell } from "../utils/nix";
import { requireTools } from "../utils/detect";

const FLAGS = "--profile wasm-release --target wasm32-unknown-unknown -p agx";

export async function buildWasm(): Promise<void> {
  await requireTools(["cargo", "wasm-bindgen"], "Run `nix develop` or add to NixOS packages");

  const cmd = [
    `cargo build ${FLAGS}`,
    `wasm-bindgen --out-dir wasm --web target/wasm32-unknown-unknown/wasm-release/agx.wasm`,
  ].join(" && ");

  const code = await runInDevShell(cmd);
  if (code !== 0) process.exit(code);
}
