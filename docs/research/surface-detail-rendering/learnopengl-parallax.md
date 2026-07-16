# Parallax Mapping — the basic taxonomy (LearnOpenGL)
https://learnopengl.com/Advanced-Lighting/Parallax-Mapping

A clean ladder of increasing cost/quality for the **per-pixel, raster-only**
family. All operate on a height/depth texture in the fragment shader and do
**not** modify geometry (silhouette is always the flat polygon's).

1. **Parallax / offset mapping** (Kaneko 2001): shift UVs by a single height
   sample × view-tangent angle. Cheapest. Artifacts: "swimming," wrong depth at
   grazing angles.
2. **Parallax with offset limiting**: clamp the offset to a max fraction to
   suppress worst swimming. Still a single sample.
3. **Steep parallax**: sample the heightfield in N discrete layers along the
   view ray; pick the first layer that drops below the surface. Gives real
   self-occlusion. Cost ∝ N (typically 10–50). Visible banding.
4. **Parallax Occlusion Mapping / POM** (Tatarchuk 2005): steep parallax +
   **linear interpolation** between the last-above and first-below samples for
   sub-step accuracy + optional **self-shadowing** rays. Industry workhorse
   (UE `BumpOffset`, Unity HDRP `Parallax`). No silhouette correction.
5. **Relief mapping** (Oliveira 2000, Policarpo 2007): iterative ray/
   heightfield intersection with **binary-search** refinement → higher quality
   than POM, higher cost. Accelerated by cone-step maps.

> Fundamental limitation of *all* these: a visual trick that does **not**
> modify geometry → **no silhouette correction**, no real cast shadows onto
> other geometry. (polycount)
