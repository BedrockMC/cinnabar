//! Content tiers match the client hardware-memory utility; see the library references.

use super::Subpack;

/// Converts physical memory bytes to vanilla's tier with strict upper boundaries.
pub(super) fn memory_tier(bytes: u64) -> u32 {
    [2, 4, 6, 8, 12]
        .into_iter()
        .filter(|gib| bytes > gib * (1 << 30))
        .count() as u32
}

/// Chooses the last highest supported tier, falling back to the pack root.
pub(super) fn select(subpacks: &[Subpack], memory: u32) -> &str {
    subpacks
        .iter()
        .filter(|pack| pack.memory_tier <= memory)
        .max_by_key(|pack| pack.memory_tier)
        .map_or("", |pack| pack.folder.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_hardware_boundaries_and_largest_tier() {
        for (tier, gib) in [2, 4, 6, 8, 12].into_iter().enumerate() {
            let bytes = gib * (1 << 30);
            assert_eq!(memory_tier(bytes - 1), tier as u32);
            assert_eq!(memory_tier(bytes), tier as u32);
            assert_eq!(memory_tier(bytes + 1), tier as u32 + 1);
        }
        assert_eq!(memory_tier(u64::MAX), 5);
    }

    #[test]
    fn automatic_selection_uses_last_supported_highest_tier() {
        let packs = [("high", 4), ("low", 0), ("medium", 2), ("last", 2)]
            .into_iter()
            .map(|(folder, memory_tier)| Subpack {
                folder: folder.into(),
                name: folder.into(),
                memory_tier,
            })
            .collect::<Vec<_>>();
        assert_eq!(select(&packs, 1), "low");
        assert_eq!(select(&packs, 2), "last");
        assert_eq!(select(&packs, 5), "high");
        assert_eq!(select(&packs[..1], 0), "");
    }
}
