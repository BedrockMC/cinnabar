# Headwear

Open `bunny_ears.bbmodel` in Blockbench with `bunny_ears.png` beside it.
Open `crown.bbmodel` with `crown.png` beside it for the rigid gold crown.
Open `devil_horns.bbmodel` for red stepped horns attached directly to the head;
its original palette texture is also embedded in the project.
All projects and textures are original Cinnabar artwork under the repository license.
After editing, run `python3 crates/render-model/src/hats/export.py` to regenerate
`bunny_ears.geo.json`, the runtime export. Add `crown` or `devil_horns` to export the other headwear.
Tests check the exports against their editable projects.

Each ear is a connected, shallow extrusion of a stepped pixel silhouette.
The ears sit directly on the head; there is no headband. Flat polygon normals
keep the faces crisp. Upper-half rings follow a chain of joints, with shared ring positions across adjacent faces. There are no internal
caps, gaps or separate rigid pieces at the joints. The lower half stays upright.
Keep each joint pivot aligned with its surface ring and parents before children.
Bake mesh transforms and group rotations into the surface before exporting.

For bunny ears, the first four texture pixels are black, highlight, pink and light-pink swatches.
Face UVs select these swatches in discrete pixel patches; keep the texture unfiltered.
