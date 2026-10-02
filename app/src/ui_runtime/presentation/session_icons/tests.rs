use super::*;

/// Distinct colored variants let packing tests detect accidental key collapse.
fn sprite(metadata: u32) -> SessionIcon {
    SessionIcon {
        identifier: Arc::from("test:variant"),
        metadata,
        width: 16,
        height: 16,
        rgba8: vec![metadata as u8; 16 * 16 * 4].into(),
    }
}

#[test]
fn metadata_variants_get_distinct_uvs_and_large_catalogs_grow_the_page() {
    let icons = SessionIcons {
        icons: (0..600).map(sprite).collect(),
        misses: HashMap::new(),
    };
    let (page, refs) = pack(&icons, 7).unwrap();
    let variants = &refs["test:variant"];
    assert_eq!(variants.len(), 600);
    assert_ne!(variants[&0].uv, variants[&599].uv);
    assert!(page.pixels().len() > (MIN_PAGE_SIDE * MIN_PAGE_SIDE * 4) as usize);
    assert_eq!(variants[&599].page, 7);
}
