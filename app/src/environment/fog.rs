use assets::RuntimeAssets;
use client_world::WorldStream;

/// Samples the current client's 27-position biome layer (Lens 0x4e429e0, table 0x1501eeff0).
pub(crate) fn fog_biome_samples(
    stream: &WorldStream,
    assets: &RuntimeAssets,
    position: [f32; 3],
) -> Vec<Option<Box<str>>> {
    const RING: [[f32; 2]; 9] = [
        [0.0, 0.0],
        [-12.0, 0.0],
        [-8.0, -8.0],
        [0.0, -12.0],
        [8.0, -8.0],
        [12.0, 0.0],
        [8.0, 8.0],
        [0.0, 12.0],
        [-8.0, 8.0],
    ];
    let [x, y, z] = position.map(f32::floor);
    let rules = &assets.biome_assets().rules;
    [0.0, -3.0, 3.0]
        .into_iter()
        .flat_map(|dy| RING.map(|[dx, dz]| [x + dx, y + dy, z + dz]))
        .map(|position| {
            let id = stream.camera_biome_id(position)?;
            let index = rules.binary_search_by_key(&id, |rule| rule.id).ok()?;
            Some(rules[index].name.clone())
        })
        .collect()
}
