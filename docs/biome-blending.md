# Biome tint sampling evidence

Reference: Lens reconstructed client **1.26.50.26**, artifact **6**.
The addresses below are artifact RVAs, not addresses in the 26.30 analysis image.
Read with `artifact_function(artifact_id=6, rva=...)`.

- `0x1de5f30`, `LatticePointColorCache::buildLatticePoints`: seven points per
  axis, spaced four blocks apart, covering offsets -12 through +12 from the
  cache origin. The raw and canonical views agree.
- `0x1de6110`: each point counts the 27 biome samples at offsets -4, 0, +4
  in X, Y and Z. It retains the four most frequent biome IDs. Each retained
  count is divided by all 27 samples; discarded IDs are not renormalized.
- `0x1de69b0`, `LatticePointColorCache::sampleColor`: takes an integer
  `BlockPos`, finds the eight nearest points from 27 candidates, and weights
  each by `1 / (distance + epsilon)`, then divides by the sum of those weights.
  Equal-distance candidates retain the X/Y/Z traversal order. The Z indexing
  adds one lattice step relative to the loop variable, so the actual candidates
  are symmetric on all axes.
  Candidate cell division truncates toward zero relative to the cache origin.
- `0x1ef8770`, `RenderChunkShared::startRebuild`: writes the cache origin as
  the render chunk's minimum block position plus eight on each axis. This is
  the geometry field passed to the snapshot and lattice builder by `0x1ee0990`.
- `read_data(artifact="6", address="0x14ffab690", type="f32")` gives epsilon
  **1.1920928955078125e-7**, exactly `f32::EPSILON`.
- `0x31aba00`: builds the biome snapshot at four-block spacing within radius
  16. Its entries start at zero; an absent chunk leaves a zero biome ID.
- `0x1ee0990`, `RenderChunkBuilder::build`: installs that snapshot and builds
  the lattice cache. Cache selection is conditional on a virtual capability
  query. Its identity has not been resolved to a user graphics setting.
- `0x1de75a0` has a direct grass-colour path when the cache is disabled and
  otherwise calls the lattice sampler. `0x6a89ae0` calls it with a block
  position and tint method 5. This proves block-position sampling for that
  caller, not every grass, foliage or water tessellation route.
- `0x1dda240` installs tint strategies 1–4 with foliage palette callbacks,
  5 with `0x1dfa500` (grass), and 6 with `0x1dfa6b0` (water). The latter two
  query the supplied lattice under the same capability test; otherwise they
  read per-column colours. `0x1df9eb0` sends all four foliage kinds through the
  lattice unless seasonal tinting applies. Its fallback averages an offset
  list through `0x731d3d0`; that list's shape remains unresolved.
- `0x6a2f110` queries a vine's tint once at its integer block position before
  writing vertex colours. This corroborates block-position queries for foliage.
- `0x1dccd10` is a separate grass averaging path: 25 horizontal samples spaced
  four blocks apart, at the supplied Y. It must not be confused with the
  lattice cache. The graphics mode selecting it is still unresolved.

## Cinnabar change and limits

The render shader already used a provisional horizontal four-block lattice;
`biome.rs`'s radius-one helper only drove CPU diagnostics. Changing its radius
alone would not change the screenshot. The shader used separable linear
weights, ignored vertical neighbours, and clamped absent neighbour samples to
its own edge. Model tint queries also varied over model geometry.

The new record caches the 3D lattice counts, carries 27 neighbour identities,
uses biome zero for absent samples, and invalidates vertical and diagonal mesh
consumers on arrival, replacement or eviction. Query weights are generated once
from Rust for the 343 signed within-cell integer positions. Uniform records
skip the lattice allocation. A nine-point-per-axis scratch grid reduces packed
biome lookups from 9,261 to 729 per nonuniform record. Model vertices share their
own block's tint query. Foliage kinds select their palettes inside the average.

**Incomplete:** this ports the verified lattice cache, not a proven universal
rule for every vanilla graphics mode and tint type. Top-four ties involving
more than four equally frequent biome IDs, all per-corner tessellation routes, the
capability-to-setting mapping, and vanilla neighbour-arrival invalidation still
need reference evidence. The owner's screenshot has no retained biome records;
its live hard edge has not been reproduced or conclusively attributed.

The PNG gallery is a CPU palette preview with synthetic artwork, not a GPU
frame or native acceptance witness. The complete-halo before preview can already
show a gradient; it cannot prove that this change fixes the owner's live seam.
No live server connection is needed or used by these tests.
