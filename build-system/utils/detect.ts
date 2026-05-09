import { $ } from "bun";
import { existsSync } from "fs";

export interface DistroInfo {
  id: string;
  isNixOS: boolean;
  packageManager: string;
}

export async function detectDistro(): Promise<DistroInfo> {
  const osRelease = await $`cat /etc/os-release 2>/dev/null || echo ""`.text().catch(() => "");
  const isNixOS = osRelease.includes("ID=nixos") || existsSync("/etc/nixos");

  let id = "unknown";
  for (const line of osRelease.split("\n")) {
    if (line.startsWith("ID=")) id = line.slice(3).replace(/"/g, "");
  }

  const pm = guessPackageManager(id);
  return { id, isNixOS, packageManager: pm };
}

function guessPackageManager(id: string): string {
  const map: Record<string, string> = {
    nixos: "nix",
    ubuntu: "apt-get",
    debian: "apt-get",
    fedora: "dnf",
    arch: "pacman",
    opensuse: "zypper",
    alpine: "apk",
    void: "xbps",
  };
  return map[id] ?? "unknown";
}

export async function toolInstalled(name: string): Promise<boolean> {
  const r = await Bun.spawnSync(["which", name]);
  return r.exitCode === 0;
}

export async function requireTools(tools: string[], hint?: string): Promise<void> {
  const missing: string[] = [];
  for (const t of tools) {
    if (!(await toolInstalled(t))) missing.push(t);
  }
  if (missing.length > 0) {
    console.error(`\n  Missing tools: ${missing.join(", ")}`);
    if (hint) console.error(`  ${hint}`);
    process.exit(1);
  }
}
