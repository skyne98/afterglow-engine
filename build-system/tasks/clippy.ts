import { $ } from "bun";

export async function runClippy(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "clippy"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}
