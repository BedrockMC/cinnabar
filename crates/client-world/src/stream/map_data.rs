//! Map images assembled from server pixel updates, for framed maps.

use std::collections::BTreeMap;

use protocol::{MAP_IMAGE_SIDE, MapDataEvent};

use super::WorldStream;

/// A client resource budget, not a gameplay limit.
pub const MAX_RETAINED_MAPS: usize = 64;

/// One map's 128x128 pixels, packed RGBA with red in the low byte; untouched pixels are zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MapImage {
    pub pixels: Vec<u32>,
    /// Changes on every applied update.
    pub revision: u64,
}

#[derive(Default)]
pub(super) struct MapImages {
    maps: BTreeMap<i64, MapImage>,
    dropped: u64,
}

impl MapImages {
    fn apply(&mut self, event: &MapDataEvent) {
        let side = MAP_IMAGE_SIDE as usize;
        if !self.maps.contains_key(&event.map_id) && self.maps.len() >= MAX_RETAINED_MAPS {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        let image = self.maps.entry(event.map_id).or_insert_with(|| MapImage {
            pixels: vec![0; side * side],
            revision: 0,
        });
        let width = event.width as usize;
        for row in 0..event.height as usize {
            let target = (event.start_y as usize + row) * side + event.start_x as usize;
            let source = row * width;
            let (Some(destination), Some(pixels)) = (
                image.pixels.get_mut(target..target + width),
                event.pixels.get(source..source + width),
            ) else {
                continue;
            };
            destination.copy_from_slice(pixels);
        }
        image.revision = image.revision.wrapping_add(1);
    }
}

impl WorldStream {
    pub(super) fn consume_map_data(&mut self, event: &MapDataEvent) {
        self.map_images.apply(event);
    }

    /// The assembled image of `map_id`, if any pixels have arrived.
    #[must_use]
    pub fn map_image(&self, map_id: i64) -> Option<&MapImage> {
        self.map_images.maps.get(&map_id)
    }

    /// Updates dropped because the retention budget was full.
    #[must_use]
    pub const fn dropped_map_updates(&self) -> u64 {
        self.map_images.dropped
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn event(map_id: i64, start_x: u32, width: u32, value: u32) -> MapDataEvent {
        MapDataEvent {
            map_id,
            start_x,
            start_y: 1,
            width,
            height: 2,
            pixels: Arc::from(vec![value; (width * 2) as usize]),
        }
    }

    #[test]
    fn updates_patch_their_rectangle_and_bump_the_revision() {
        let mut images = MapImages::default();
        images.apply(&event(3, 4, 2, 0xAABBCCDD));
        images.apply(&event(3, 0, 1, 0x11));
        let image = &images.maps[&3];
        let side = MAP_IMAGE_SIDE as usize;
        assert_eq!(image.pixels[side + 4], 0xAABBCCDD);
        assert_eq!(image.pixels[2 * side + 5], 0xAABBCCDD);
        assert_eq!(image.pixels[side], 0x11);
        assert_eq!(image.pixels[0], 0);
        assert_eq!(image.revision, 2);
    }

    #[test]
    fn new_maps_past_the_budget_are_dropped_but_known_maps_keep_updating() {
        let mut images = MapImages::default();
        for id in 0..MAX_RETAINED_MAPS as i64 {
            images.apply(&event(id, 0, 1, 1));
        }
        images.apply(&event(1_000, 0, 1, 1));
        assert!(!images.maps.contains_key(&1_000));
        assert_eq!(images.dropped, 1);
        images.apply(&event(0, 0, 1, 2));
        assert_eq!(images.maps[&0].revision, 2);
    }
}
