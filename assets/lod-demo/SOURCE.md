# LOD demo source

`Avocado.gltf`, `Avocado.bin`, and the upstream `README.md` come from the
[Khronos glTF Sample Assets Avocado model](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Avocado).
The model is Microsoft CC0 1.0 Universal. Texture images are intentionally not
vendored: the static LOD cook consumes geometry only and the demo uses an
engine material.

Regenerate the checked-in runtime container with:

```sh
cargo run -p afterglow-pipeline -- \
  static-lod assets/lod-demo/Avocado.gltf \
  crates/afterglow-web/web/assets/lod-demo.big
```
