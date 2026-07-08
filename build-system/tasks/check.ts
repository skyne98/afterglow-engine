import { runInDevShell } from "../utils/nix";

export async function runCheck(): Promise<void> {
  process.exit(await runInDevShell("cargo check"));
}
