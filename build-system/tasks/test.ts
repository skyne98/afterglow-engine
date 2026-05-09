import { $ } from "bun";

export async function runTest(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "test"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}
