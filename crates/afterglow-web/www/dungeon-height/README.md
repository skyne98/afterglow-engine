# Dungeon resident POM height fields

These are the 1K ambient-occlusion PNGs from the same ambientCG materials used
by `dungeon.big`, renamed as resident pseudo-height fields:

- `Rock064_Height.png`
- `Ground103_Height.png`
- `PavingStones150_Height.png`

Source archives: `https://ambientcg.com/get?file=<name>_1K-PNG.zip`, member
`<name>_1K-PNG_AmbientOcclusion.png`.

ambientCG assets are CC0/public domain. The files are intentionally resident:
a non-uniform POM march must not depend on asynchronous VT page residency.
Together they occupy about 1.7 MiB on disk while the 8K PBR channels remain
virtual.
