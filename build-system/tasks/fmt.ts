import { runInDevShell } from "../utils/nix";

export async function runFmt(): Promise<void> {
  process.exit(await runInDevShell("cargo fmt"));
}
