import { $ } from "bun";
import { detectDistro, toolInstalled } from "./detect";

export interface NixEnv {
  available: boolean;
  flakeRoot: string;
}

export function flakeRoot(): string {
  return new URL("../../", import.meta.url).pathname;
}

export async function checkNix(): Promise<NixEnv> {
  const hasNix = await toolInstalled("nix");
  return { available: hasNix, flakeRoot: flakeRoot() };
}

export async function inNixShell(): Promise<boolean> {
  return process.env.IN_NIX_SHELL === "1" || process.env.IN_NIX_SHELL === "pure";
}

export async function runInDevShell(command: string): Promise<number> {
  const env = process.env;
  const isNix = (await checkNix()).available;
  const already = await inNixShell();

  if (already || !isNix) {
    const r = Bun.spawnSync(["bash", "-c", command], { stdio: ["inherit", "inherit", "inherit"], env });
    return r.exitCode ?? 1;
  }

  const flake = flakeRoot();
  const r = Bun.spawnSync(
    ["nix", "develop", `${flake}#default`, "--command", "bash", "-c", command],
    { stdio: ["inherit", "inherit", "inherit"], env }
  );
  return r.exitCode ?? 1;
}

export async function doctorNix(): Promise<void> {
  const { available } = await checkNix();
  const distro = await detectDistro();
  console.log(`  Distro:     ${distro.id}${distro.isNixOS ? " (NixOS)" : ""}`);
  console.log(`  Nix:        ${available ? "✓" : "✗ not found"}`);
  console.log(`  Flake root: ${flakeRoot()}`);
  console.log(`  In nix shell: ${(await inNixShell()) ? "yes" : "no"}`);

  const needed = ["cargo", "rustc"];
  if (!distro.isNixOS) needed.push("pkg-config");
  const missing: string[] = [];
  for (const t of needed) {
    if (!(await toolInstalled(t))) missing.push(t);
  }
  if (missing.length > 0) {
    console.error(`\n  Missing core tools: ${missing.join(", ")}`);
    if (distro.isNixOS) {
      console.error("  → Run: nix develop or add them to your NixOS config");
    } else {
      console.error(`  → Install via ${distro.packageManager}`);
    }
  }
}
