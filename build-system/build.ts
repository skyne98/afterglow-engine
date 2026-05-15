#!/usr/bin/env bun
import { doctorNix } from "./utils/nix";
import { detectDistro, requireTools } from "./utils/detect";
import { runDev, runRelease } from "./tasks/native";
import { buildWasm } from "./tasks/wasm";
import { devServe } from "./tasks/dev-wasm";
import { runCheck } from "./tasks/check";
import { runTest } from "./tasks/test";
import { runFmt } from "./tasks/fmt";
import { runClippy } from "./tasks/clippy";

const help = `
  afterglow build system

  usage:
    bun run build-system/build.ts <command>

  commands:
    native          cargo run -p agx
    native-release  cargo run -p agx --release
    build-wasm      build wasm (nix develop)
    wasm            dev server with live reload on :4000
    check           cargo check
    test            cargo test
    fmt             cargo fmt
    clippy          cargo clippy
    doctor          check system + tooling

  examples:
    bun build-system/build.ts native
    bun build-system/build.ts serve-wasm
`;

const commands: Record<string, (args: string[]) => Promise<void>> = {
  native: runDev,
  "native-release": runRelease,
  "build-wasm": buildWasm,
  wasm: devServe,
  check: runCheck,
  test: runTest,
  fmt: runFmt,
  clippy: runClippy,
};

async function doctor(): Promise<void> {
  const distro = await detectDistro();
  console.log("\n  afterglow environment");
  console.log(`  ${"=".repeat(40)}`);
  console.log(`  Distro:     ${distro.id}${distro.isNixOS ? " (NixOS)" : ""}`);
  console.log(`  Packages:   ${distro.packageManager}`);
  await doctorNix();
  process.exit(0);
}

async function main() {
  const cmd = process.argv[2];

  if (!cmd || cmd === "--help" || cmd === "-h") {
    console.log(help);
    process.exit(cmd ? 0 : 1);
  }

  if (cmd === "doctor") {
    await doctor();
    return;
  }

  const fn = commands[cmd];
  if (!fn) {
    console.error(`\n  Unknown command: "${cmd}"\n${help}`);
    process.exit(1);
  }

  await fn(process.argv.slice(3));
}

main().catch((e) => {
  console.error("Fatal:", e);
  process.exit(1);
});
