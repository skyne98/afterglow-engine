import { runInDevShell } from "../utils/nix";

export async function runTest(): Promise<void> {
  const code1 = await runInDevShell("cargo test");
  if (code1 !== 0) process.exit(code1);

  const code2 = await runInDevShell(
    "cargo test -p afterglow-engine --features test-support"
  );
  process.exit(code2);
}
