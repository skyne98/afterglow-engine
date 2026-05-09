import { $ } from "bun";

export async function runFmt(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "fmt"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}
