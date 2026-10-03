use super::super::*;

impl WorldStream {
    pub(in super::super) fn replace_biome_definitions(
        &mut self,
        definitions: Arc<[BiomeDefinitionEvent]>,
    ) {
        #[cfg(debug_assertions)]
        {
            const MAX_SEASONAL_DIAGNOSTICS: usize = 32;
            let eligible = definitions.iter().filter(|row| {
                row.temperature < assets::SEASONAL_FOLIAGE_COLD_THRESHOLD || row.snow_foliage > 0.0
            });
            let count = eligible.clone().count();
            eprintln!(
                "BIOME_SEASONS definitions={} candidates={} listed={}",
                definitions.len(),
                count,
                count.min(MAX_SEASONAL_DIAGNOSTICS)
            );
            for row in eligible.take(MAX_SEASONAL_DIAGNOSTICS) {
                eprintln!(
                    "BIOME_SEASONS name={} id={:?} temperature={} max_snow={:?} snow_foliage={}",
                    row.name,
                    row.biome_id,
                    row.temperature,
                    row.max_snow_accumulation,
                    row.snow_foliage
                );
            }
        }
        let retaining_rows = Arc::ptr_eq(&self.biome_definitions, &definitions);
        let live: Vec<_> = definitions
            .iter()
            .enumerate()
            .map(|(index, row)| LiveBiomeDefinition {
                name: &row.name,
                biome_id: row.biome_id,
                temperature: row.temperature,
                downfall: row.downfall,
                snow_foliage: if retaining_rows {
                    self.seasonal_foliage
                        .snow
                        .get(index)
                        .copied()
                        .unwrap_or(row.snow_foliage)
                } else {
                    row.snow_foliage
                },
                max_snow_accumulation: row.max_snow_accumulation,
                map_water_argb: row.map_water_color,
            })
            .collect();
        let Ok(resolved) = self.runtime_assets.biome_assets().resolve_live(&live) else {
            self.record_normalization_error(
                NormalizationErrorReason::BiomeDefinitionResolutionFailure,
            );
            return;
        };
        for _ in 0..resolved.skipped_definitions {
            self.record_normalization_error(
                NormalizationErrorReason::BiomeDefinitionResolutionFailure,
            );
        }
        let Some(next_revision) = self.biome_tint_revision.checked_add(1) else {
            self.record_normalization_error(NormalizationErrorReason::BiomeTintRevisionOverflow);
            return;
        };
        self.biome_tint_revision = next_revision;
        if !retaining_rows {
            self.seasonal_foliage.reset(&definitions);
        }
        self.biome_definitions = definitions;
        self.resolved_biome_tints = Arc::new(resolved);
        self.invalidate_resident_biome_tints(Instant::now());
    }
}
