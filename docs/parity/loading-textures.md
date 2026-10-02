# Loading-screen texture residency

The loading screen must paint against the texture pages it publishes. A completed
artwork atlas is installed before painting; server-page writes made during painting
are uploaded afterward without installing another artwork atlas. Superseding an
artwork request discards its prepared result. Replacing a pack clears its old art
references, and changed bytes under the same texture key invalidate decoded artwork.

Cold texture misses decode in increasing source area, with a stable key tie-break.
The decode budget starts after source lookup. This lets small backdrops and animation
strips become resident before a large logo consumes the inline budget.

## Vanilla references

- Lens `L:1.26.50.26:0x4d571c0` reads `tiled_scale` and identifies
  `SpriteComponent::setTiledScale`; `L:1.26.50.26:0x4d56ee0` reads gradient
  direction and its two colors. `L:1.26.50.26:0x4d33810` identifies
  `UIAnimationComponent::_createAnimation` and reads flip-book frame parameters.
  The analysis service was unavailable; these were read through the source-backed
  artifact catalog after an `analysis_batch` attempt.
- mcsrc `R:s/SpriteComponent.cpp:3357` multiplies tile dimensions by the tile scale;
  `R:g/GradientRenderer.cpp:112` selects vertical or horizontal gradient drawing.
  `R:w/WorldGenerationProgressHandler.cpp:253` selects the building-terrain message.
- The pinned vanilla pack's `ui/progress_screen.json:1215` declares the 2× tiled
  overworld dirt backdrop and black gradient alpha 0.5 to 0.7.
  `ui/ui_common.json:2244` supplies the tiled image base. The flip-book at
  `ui/progress_screen.json:531` uses ten 64×8 cells at 10 fps, reversing direction,
  with a 0.7 tint. `texts/en_US.lang:8153` and `:8179` supply the screen's wording.

## Regression evidence

`forms/loading_texture_tests.rs` renders through the real carrier and local pack.
The Zeqa fixture checks cold dirt residency, actual dirt pixels and gradient
darkening, identical screen pixels across an artwork repack and pack reload, and
same-key replacement with different artwork bytes. A synthetic green artwork makes
an incorrect region obvious: restoring the old publication order changes both the
title and loading-bar area to green. The cold-atlas test fails with the old decode
order because dirt is not resident after the expensive title decode.

Zeqa's actual `textures/ui/loading_bar.png` is fully transparent. The unmodified
pack must keep that override; the small figures in the owner's corrupted frame
are unrelated atlas pixels. A separate offline fixture removes only that override
and checks vanilla bar pixels and animation. No pack files are changed on disk.

The requested `zeqapacks` archives contain duplicate entity/art packs without the
UI title. The existing `scratchpad/zq/ui` layer used by the earlier render script
provides the Zeqa UI fixture. Pack files and screenshots remain outside git.

The art-page publication issue dates to `f4a4d802`, with the asynchronous end-of-frame
installation added in merge `ad949ed8`. Unordered misses became visible omissions
with the decode budget in `ea2ec797`. The worker in `153d8ebe` retained superseded
prepared results and cached pack artwork by key rather than payload; live reloads
in `c1381a21` exposed the latter. No separate fault in `ec9b3731` was established.

The CPU snapshot rasterizer now interpolates vertex colors, so gradient assertions
measure the same interpolation as the UI shader. Previously it used the first
vertex's color for an entire triangle. Offline evidence does not close a live
visual parity gate or prove how long the owner's black frame persisted.
