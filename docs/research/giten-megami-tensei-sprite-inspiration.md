# Giten Megami Tensei: Tokyo Revelation — sprite source & inspiration audit

> Research / inspiration note for the Afterglow **art + rendering** direction.
> This records the *source* of a hand-extracted reference sprite set (a specific
> extractable dataset, both game versions), the *format findings* that made the
> extraction possible, and the *visual inspiration* the set is meant to supply to
> the engine's aesthetic. The extracted assets themselves are **not** checked
> into this repo (copyright + ~108 MB); they live locally at `~/Downloads/giten`
> and are referenced below so the work can be reproduced on demand.

**Research date:** 2026-08-25

---

## 0. What this is

Giten Megami Tensei: Tokyo Revelation (偽典・女神転生 東京黙示録, "Giten") is a
1997 (PC-98) / 1999 (Windows) dungeon-crawl RPG — a spin-off of Atlus' *Shin
Megami Tensei* — known for its dark cyberpunk aesthetic, hand-drawn monsters,
and dense demon-encyclopedia roster. It is abandonware distributed freely
(no official Western release), which is why a full, verified sprite extraction
is feasible and useful as reference art.

For Afterglow this is an **inspiration/source dataset**: a large catalog of
coherent, palette-constrained, multi-frame pixel-art creatures to study for
procedural style, sprite/animation conventions, and UI/economy of detail.

---

## 1. Source of the sprites (where the game comes from)

| Version | Year | Platform | Location |
|---------|------|----------|----------|
| Windows | 1999 | Windows 9x (DirectDraw) | Internet Archive: `archive.org/details/giten-megami-tensei-tokyo-mokushiroku` (raw CD image, `DDSWin.img`) |
| PC-98   | 1997 | NEC PC-9801 | Internet Archive: `archive.org/details/giten-megami-tensei` (raw CD image, `.bin/.cue`) |

Both are raw 2352-byte mode-1 CD images. Additional community context:
- **CDRomance** hosts an English-patched PC-98 copy (`.bin/.cue`).
- **Megami Tensei Wiki** (`megatenwiki.com`) catalogs the game; its PC-98 sprite
  category (602 PNGs) was ripped via **emulator RAM dumps** — i.e. even the
  community could not cleanly decode the PC-98 sprite container from the files
  themselves (see §3).
- The game's own fan **"Demon Encyclopedia"** tool (`devil.exe`, from
  `Patch_Compendium.rar` in the Windows image) reads the exact same `P`/`FC`/`ET`
  data as this extraction to display inline monster stats **and** images — and
  its docs state images are **not** displayed on the NEC98 build, independent
  confirmation that the PC-98 `FC` format is not directly readable.

---

## 2. Data layout (Windows build) — what maps to what

Windows install splits assets into per-purpose folders under `ddswin/`:

| Folder | Content | Verdict |
|--------|---------|---------|
| `fc/` (1577 files) | Character / enemy **sprites** | 🎨 visuals (extracted) |
| `w/` (30 files) | Wall / background **textures** | 🎨 visuals (extracted) |
| `et/` (362) | Event **scripts / tables** | data |
| `p/` (432) | Monster **stat/parameter** data, 1 per enemy | data |
| `m/` (309) | Message / script data | data |
| `s/` (74) | Sound / MIDI | data |

Each file in `fc/` is a **concatenated stack of 8-bit 256×256 BMPs** — one per
animation frame (1–52 frames). The demon-encyclopedia tool reads `P` + `FC` + `ET`
together, which is what let us confirm `fc/` = the sprite images.

The PC-98 disc (`DDS98/`) carries the same asset families (`CA`, `FC`, `M`, `P`,
`ET`, `MS`, `ID`, `SB`, `SM`, `ST`, `SE`) with `CA`/`P` byte-identical to the
Windows versions.

---

## 3. Format findings (why extraction works, and where it stops)

### Windows `fc/` — fully decoded ✅
- Each file: N × (14-byte file header + 40-byte BITMAPINFOHEADER + 8-bit palette
  + 256×256 indexed pixels), frames concatenated back-to-back.
- Decode = find `BM` boundaries, read each BMP with Pillow, save each frame as PNG.
- Result: **6344 frames across 1577 groups** + **30 walls**, all verified.

### PC-98 `FC*.BIN` — proprietary container, not cleanly decodable ⚠️
- Multi-block container: 12-byte header (u32 region size, u16 count, u16 table
  length) + offset table (u16 offsets to sub-objects/frames) + tilemap byte-pairs
  (`<tile_index>, <attr 0x10/0x20/0x30>`) + 12-bit palettes + RLE/LZSS-compressed
  4-bpp plane data + signed anchors.
- Recovering pixels requires disassembling the 16-bit `DDS98.EXE` (confirmed to
  read `fc%.4x.bin`) to extract the proprietary decompressor. No community
  extractor exists; the wiki rips came from emulator RAM dumps.
- So: the **complete sprite content is captured in the Windows extraction**, and
  the PC-98 build is a documented, reproducible research puzzle.

---

## 4. Extraction pipeline (reproducible)

All in `~/Downloads/giten/` (`tools/` holds the scripts):

1. **Convert raw CD → ISO**: strip 16-byte mode-1 sector headers from each
   2352-byte sector (`tools/bin2iso.py`), then `7z x`.
2. **Split BMP frame stacks → PNG** (`tools/fc_convert.py`):
   `out_win/fc/<name>/frameNN.png` + `out_win/w/`.
3. **Web viewer** — single self-contained HTML with every frame embedded:
   - `tools/make_data.py` → lossless-WebP full-res frames + animated GIF previews
     as data URIs (`viewer/data.js`).
   - `viewer-src/` = a **Vue 3 + `@zoom-image/vue`** app (search, filter,
     fullscreen pixelated pan/zoom, frame filmstrip).
   - Assembled into `viewer/giten_sprites.html` (65 MB, works from `file://`).

Extracted dataset: **6374 full-resolution frames + 30 walls**, ~108 MB at
`~/Downloads/giten/out_win/`.

---

## 5. Inspiration for Afterglow

Giten is a masterclass in doing a lot with a little, which matters for an engine
whose goals include low-end renderers, procedural style, and tight budgets:

- **Palette constraint as identity.** Everything is 8-bit index-color; individual
  sprites use a handful of graded hues. Worth studying for procedural/dithering
  palettes and for `assets/` test scenes with a strong, restricted look.
- **Frame economy.** Animations are short (1–10 frames typical), looping the
  same offset by ±1px. A good reference for skeletal vs. frame-sprite tradeoffs
  and for cheap "alive" motion.
- **Monster-design silhouette.** Large readable silhouettes with a few accent
  colors — a useful target for procedural creature/character generation and for
  character-creator style direction.
- **Urban-apocalyptic tone.** Dark cyberpunk + occult subject matter informs the
  mood we may want in demo/art scenes (the dungeon-height demo and painter RAD
  are adjacent directions).
- **Reference workflow.** The whole pipeline (raw image → BMP stack → frames →
  self-contained viewer) is a reusable pattern for bringing any abandonware art
  into the engine as a browsable, seeded reference set.

---

## 6. Status & follow-ups

- [x] Windows sprite set fully extracted + verified (6374 frames + 30 walls).
- [x] Self-contained Vue 3 viewer with pan/zoom.
- [x] PC-98 container reverse-engineered to structure level + documented.
- [ ] (optional) Disassemble `DDS98.EXE` to recover the PC-98 FDI/F-BASIC
      decompressor → native PC-98 sprite PNGs.
- [ ] (optional) If used as in-engine art, re-seat a *subset* (or procedurally
      derived style set) under `assets/` with a `SOURCE.md` + license scan —
      **not** the full copyrighted set.
