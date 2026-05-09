import { $ } from "bun";

export async function runDev(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "run", "-p", "agx"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}

export async function runRelease(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "run", "-p", "agx", "--release"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}
