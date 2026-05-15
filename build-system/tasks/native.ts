export async function runDev(args: string[] = []): Promise<void> {
  const r = Bun.spawnSync(["cargo", "run", "-p", "agx", "--", ...args], {
    stdio: ["inherit", "inherit", "inherit"],
  });
  process.exit(r.exitCode ?? 1);
}

export async function runRelease(args: string[] = []): Promise<void> {
  const r = Bun.spawnSync(["cargo", "run", "-p", "agx", "--release", "--", ...args], {
    stdio: ["inherit", "inherit", "inherit"],
  });
  process.exit(r.exitCode ?? 1);
}
