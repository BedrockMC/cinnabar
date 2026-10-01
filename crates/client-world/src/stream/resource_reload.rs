use super::*;

impl WorldStream {
    /// Cinnabar extension: replace visuals without resetting the live world's network sequence.
    pub fn reload_resource_assets(&mut self, assets: Arc<RuntimeAssets>) {
        if Arc::ptr_eq(&self.runtime_assets, &assets) {
            return;
        }
        let biomes_changed = self.runtime_assets.biome_assets() != assets.biome_assets();
        self.runtime_assets = assets;
        if biomes_changed {
            self.apply_immediate(
                WorldEvent::BiomeDefinitions(protocol::BiomeDefinitionsEvent {
                    definitions: self.biome_definitions.clone(),
                }),
                None,
            );
        }
        let now = Instant::now();
        let resident: Vec<_> = self.resident.iter().copied().collect();
        for key in resident {
            self.mark_dirty_exact(key, now);
        }
    }
}
