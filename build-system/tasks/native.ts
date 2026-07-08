import { runInDevShell } from "../utils/nix";

export async function runDev(args: string[] = []): Promise<void> {
  const cmd = `cargo run -p agx -- ${args.map(a => `'${a}'`).join(" ")}`;
  process.exit(await runInDevShell(cmd));
}

export async function runRelease(args: string[] = []): Promise<void> {
  const cmd = `cargo run -p agx --release -- ${args.map(a => `'${a}'`).join(" ")}`;
  process.exit(await runInDevShell(cmd));
}
