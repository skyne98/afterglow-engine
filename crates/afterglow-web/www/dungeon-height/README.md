# Dungeon resident POM height fields

These are the official 1K, 16-bit displacement PNGs from the same ambientCG
materials used by `dungeon.big`, renamed as resident height fields:

- `Rock064_Height.png`
- `Ground103_Height.png`
- `PavingStones150_Height.png`

Source archives: `https://ambientcg.com/get?file=<name>_1K-PNG.zip`, member
`<name>_1K-PNG_Displacement.png`.

ambientCG assets are CC0/public domain. The files are intentionally resident:
a non-uniform POM march must not depend on asynchronous VT page residency.
They are physical-height inputs (white/exposed, black/recessed); AO is lighting
information and is not a valid substitute for geometric height.
