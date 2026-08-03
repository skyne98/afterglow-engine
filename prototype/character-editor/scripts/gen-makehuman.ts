#!/usr/bin/env bun
/**
 * Generate both genital-proxy bodies (male + female) with the body-morph library
 * transferred onto the proxy topology at bake time (so the runtime editor does
 * NO refit - the proxy mesh carries genitals + all morphs as native glTF targets).
 *
 * Requires:
 *   - Blender 5.x on PATH
 *   - MPFB extension at ~/.config/blender/5.2/extensions/user_default/mpfb
 *   - PunkElvs CC-BY proxies in assets/character-rig/downloads/proxies/
 *   - CC0 face packs in assets/character-rig/downloads/functional/
 *
 * Output: public/character_male.glb + character_female.glb (+ sidecars)
 */
import { $ } from 'bun';
import { existsSync, renameSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const script = join(here, 'gen-proxy-transfer.py');
const dir = join(root, 'public');
const downloads = join(root, '..', '..', 'assets', 'character-rig', 'downloads');
const proxies = join(downloads, 'proxies');
const faceTargets = join(downloads, 'functional');

for (const sex of ['male', 'female']) {
  const out = join(dir, `character_${sex}.glb`);
  const temporary = join(dir, `.character_${sex}.tmp.glb`);
  const temporaryMorphs = temporary.replace('.glb', '.morphs.json');
  const temporaryControls = temporary.replace('.glb', '.controls.json');
  for (const path of [temporary, temporaryMorphs, temporaryControls]) rmSync(path, { force: true });

  console.log(`Blender proxy-transfer (${sex}) -> ${out}`);
  await $`blender --background --python-exit-code 1 --python-expr ${`exec(open('${script}').read())`}`
    .env({ ...process.env, SEX: sex, FACE_TARGET_ROOT: faceTargets,
           PROXY_ROOT: join(proxies, `punkelvs_${sex}`), CHAR_OUT: temporary })
    .cwd(root)
    .quiet();

  for (const path of [temporary, temporaryMorphs, temporaryControls]) {
    if (!existsSync(path)) throw new Error(`Blender did not create ${path}`);
  }
  renameSync(temporary, out);
  renameSync(temporaryMorphs, out.replace('.glb', '.morphs.json'));
  renameSync(temporaryControls, out.replace('.glb', '.controls.json'));
}
console.log('Done: character_male.glb + character_female.glb (+ morph sidecars) in public/.');
