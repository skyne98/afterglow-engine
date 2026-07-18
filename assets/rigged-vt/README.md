# Rigged VT demo source

`model.glb` is the unmodified self-contained download of **Decraniated (Low
Poly Retro Pixel)** by KallMor. See `LICENSE.txt` for the required CC BY 4.0
attribution and source URL. `model-2.gltf` plus `scene.bin` and `textures/` are
the unmodified external-package files for **Sci-Fi Character - Dragon Warrior
(Futuristic)** by Spooky Iluha; see `LICENSE-DRAGON.txt` (CC BY-NC 4.0). It is
selected with **2** in the demo (**1** returns to the first rig).

The normal asset pipeline handles it as one ordinary GLB input:

```sh
nix-shell shell.nix --run \
  "cargo run -p afterglow-pipeline --release -- process assets/rigged-vt crates/afterglow-web/web/assets/rigged-vt.big"
```

The pipeline embeds external glTF side files into a self-contained GLB, packs
each complete GLB as a seekable raw model asset, and extracts all images into independently paged/UASTC virtual textures named
`model.glb#image-N`. Runtime reads `model.glb` from the same `.big`, parses the
rig and animation with GLTFLoader, and sends triangle-index optimization through
the meshopt worker without changing vertex identity, skin attributes, morph
targets, skeletons, or animation tracks.
