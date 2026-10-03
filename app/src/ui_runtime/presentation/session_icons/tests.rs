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

#[test]
fn stack_icon_identity_retains_loaded_projectile_and_local_frame_override() {
    for projectile in ["minecraft:arrow", "minecraft:firework_rocket"] {
        let frame = crate::item_use::crossbow_animation_frame(None, 0, Some(projectile), false);
        assert_eq!(
            UiPresentationRuntime::item_icon_key("minecraft:crossbow", 73, Some(projectile), None),
            ("minecraft:crossbow_pulling", frame - 1),
        );
        assert_eq!(
            UiPresentationRuntime::item_icon_key(
                "minecraft:crossbow",
                73,
                Some(projectile),
                Some(0)
            ),
            ("minecraft:crossbow", 73),
        );
    }
    assert_eq!(
        UiPresentationRuntime::item_icon_key("minecraft:crossbow", 73, None, None),
        ("minecraft:crossbow", 73),
    );
    assert_eq!(
        UiPresentationRuntime::item_icon_key(
            "minecraft:stone",
            2,
            Some("minecraft:arrow"),
            Some(1)
        ),
        ("minecraft:stone", 2),
    );
}

#[test]
fn review_duplicate_icon_selection_precedes_height_sorting() {
    let first = sprite(0);
    let mut second = sprite(0);
    second.height = 32;
    second.rgba8 = vec![9; 16 * 32 * 4].into();
    let icons = SessionIcons {
        icons: vec![first, second],
        misses: HashMap::new(),
    };
    let (_, refs) = pack(&icons, 7).unwrap();
    let uv = refs["test:variant"][&0].uv;
    assert_eq!(uv[3] - uv[1], 16);
}

#[test]
fn review_oversized_icons_are_rejected_before_gutter_arithmetic() {
    let invalid = SessionIcon {
        width: u32::MAX,
        height: u32::MAX,
        rgba8: Box::new([]),
        ..sprite(1)
    };
    let icons = SessionIcons {
        icons: vec![invalid, sprite(0)],
        misses: HashMap::new(),
    };
    let (_, refs) = pack(&icons, 7).unwrap();
    assert!(refs["test:variant"].contains_key(&0));
    assert!(!refs["test:variant"].contains_key(&1));
}
