# Image clarity and sampling

Vanilla rules for the pinned Bedrock version are separated below from evidence that still
needs a matching platform capture. The desktop sampling implementation uses the established
rules; this document does not close the cross-platform visual parity gate.

| Area | Vanilla rules | Evidence limit |
| --- | --- | --- |
| Anti-aliasing control | Video names the slider **Anti-Aliasing**. Values represent positive power-of-two sample counts; `1` is single sampling. Stops come from renderer capabilities. | A universal fixed maximum is incorrect. The inspected capability path can restrict values above four samples. |
| Default sample count | The shared default is `2`; the handheld platform class uses `1`. An Education-specific operating-system branch also uses `1`. | The desktop reference resolves to `2`. Exact Windows retail, Android, iOS, Xbox, PlayStation and Switch defaults and device exceptions have not all been independently verified. |
| World AA | Gameplay color has an explicit MSAA resolve before post-processing. The inspected raster path provides no evidence for a full-screen FXAA pass. | Matching captures are still required for every platform and for exact terrain/entity/hand/UI coverage boundaries. |
| UI AA | UI composition has its own rendering stage. World sample count alone does not establish the sample count of every UI material. | Exact version-matched UI geometry coverage remains open; see [inventory rendering](inventory-gui-geometry.md). |
| Enhanced graphics | A gameplay color resolve precedes post-processing. | Cinnabar's Enhanced mode is a custom path; it does not claim Vibrant Visuals or ray-tracing parity. Their temporal/upscaling AA policies remain unverified. |
| Terrain filtering | The raster terrain atlas uses nearest magnification, nearest minification, linear interpolation between mip levels, and clamp addressing. Terrain atlas anisotropy is disabled. The anisotropic sampler used for the lightmap is a separate binding. | Atlas tile repetition is represented by repeated per-layer UVs in Cinnabar's texture arrays. |
| Terrain mips | `textures/terrain_texture.json` declares four mip levels. Atlas mips average source channel bytes over the original texel footprint, with truncation and no alpha-coverage rescaling. | Texture-pack mip declarations and texture size determine the available levels. |
| Terrain mip bias | No additional bias is established by the inspected terrain sampler. | Platform shader bias and deferred/upscaling bias are unverified; adding a negative bias is not a proven vanilla correction. |
| Entities | Actor color sampling comes from its material's sampler description. | A universal terrain-style sampler, mip chain or anisotropy level cannot be inferred for all entity materials. |
| Held items | The inspected ordinary item-in-hand color binding uses point filtering, including mip selection. | Specialized item materials, entity skins and their texture-loading mip policies require separate confirmation. |
| Item icons | `textures/item_texture.json` does not declare terrain's mip count. Flat icons retain their authored texels. | Exact target-version icon/model material sampling and coverage remain open; nearby-version point-sampling evidence is documented in [inventory rendering](inventory-gui-geometry.md). |
| Filtering settings | The inspected Video UI has no anisotropy or mip-bias control. | This does not prove every platform or graphics mode lacks additional controls. |
| Texel anti-aliasing | The UI includes a **Texel Anti-Aliasing** toggle gated by a capability. The inspected desktop capability disables it. Material configuration gates texel AA separately from MSAA and alpha-to-coverage. | The shared default also contains a console-class condition; the exact shader algorithm and platform exposure are not yet established. Enabling MSAA alone does not establish either texel AA or alpha-to-coverage. |

Cinnabar removes FXAA, keeps nearest terrain texels within each mip, and selects MSAA from the
intersection supported by its color and depth attachments. The Video slider shows only usable
sample counts and retains the saved preference across changes in device capabilities. Its
numeric label and keyboard/pointer stops use the same sample list. Sampling changes reach the
camera before that frame publishes hand geometry.

All main-pass pipelines and private depth targets must match the selected sample count.
Post-processing reads resolved color; depth consumers must explicitly reduce multisampled
depth. Color resolve must preserve the intended gamma-space terrain blending. UI composition
remains a separate single-sample layer. MSAA must not introduce a full-screen texture filter.

The gamma-compositing path currently seeds coverage samples from resolved opaque color.
It preserves sample depth but not the original opaque color of every sample where silhouettes
overlap transparent geometry. Shadow and enhanced post-process writeback also operate on
resolved color. Exact per-sample composition remains an incomplete parity gate.

The Mac capture validates Cinnabar's Metal path, not a native macOS retail Bedrock release.
Enhanced remains hard-disabled; its attachment changes are not live-rendering acceptance.
No matching vanilla comparison or console/mobile hardware capture is implied by those results.
See `plan.md` for remaining platform, image and performance gates.
