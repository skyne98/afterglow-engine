import { runInDevShell } from "../utils/nix";

export async function runClippy(): Promise<void> {
  process.exit(await runInDevShell("cargo clippy"));
}
