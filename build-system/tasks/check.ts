import { $ } from "bun";

export async function runCheck(): Promise<void> {
  const r = Bun.spawnSync(["cargo", "check"], { stdio: ["inherit", "inherit", "inherit"] });
  process.exit(r.exitCode ?? 1);
}
