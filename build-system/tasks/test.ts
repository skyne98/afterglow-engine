export async function runTest(): Promise<void> {
  const normal = Bun.spawnSync(["cargo", "test"], { stdio: ["inherit", "inherit", "inherit"] });
  if ((normal.exitCode ?? 1) !== 0) {
    process.exit(normal.exitCode ?? 1);
  }

  const testSupport = Bun.spawnSync(["cargo", "test", "-p", "afterglow-engine", "--features", "test-support"], {
    stdio: ["inherit", "inherit", "inherit"],
  });
  process.exit(testSupport.exitCode ?? 1);
}
