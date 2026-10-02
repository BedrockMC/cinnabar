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
        ..Default::default()
    };
    let packed = pack(&icons, 7).unwrap();
    let variants = &packed.refs["test:variant"];
    assert_eq!(variants.len(), 600);
    assert_ne!(variants[&0].uv, variants[&599].uv);
    assert!(packed.page.pixels().len() > (MIN_PAGE_SIDE * MIN_PAGE_SIDE * 4) as usize);
    assert_eq!(variants[&599].page, 7);
}

// A custom block item with a cube sheet draws as the GUI cube vanilla block items draw, sampling
// the sheet rather than scaling its flat thumbnail; an item without one stays a flat sprite.
#[test]
fn block_sheet_items_draw_the_gui_cube_over_their_thumbnail() {
    use ui::{UiNode, UiNodeId, UiPoint, UiRect, UiVisual};
    let mut presentation = UiPresentationRuntime::new(super::super::tests::fixture_font()).unwrap();
    presentation.gui_models.enabled = true;
    let thumbnail = |identifier: &str| SessionIcon {
        identifier: Arc::from(identifier),
        metadata: 0,
        width: 32,
        height: 32,
        rgba8: vec![9; 32 * 32 * 4].into(),
    };
    let [width, height] = assets::BLOCK_ITEM_SHEET_SIZE.map(u32::from);
    let icons = Arc::new(SessionIcons {
        icons: vec![thumbnail("test:controller"), thumbnail("test:gem")],
        block_sheets: vec![SessionIcon {
            identifier: Arc::from("test:controller"),
            metadata: 0,
            width,
            height,
            rgba8: vec![255; (width * height * 4) as usize].into(),
        }],
        misses: HashMap::new(),
    });
    observe(&mut presentation, Some(&icons));
    let bounds = UiRect::new(
        UiPoint::new(0.0, 0.0).unwrap(),
        UiPoint::new(16.0, 16.0).unwrap(),
    )
    .unwrap();
    let slot = |icon: IconRef| {
        UiNode::new(UiNodeId::new(1), None, bounds).with_visual(UiVisual::Sprite {
            texture_page: icon.page,
            uv: icon.uv,
            color: [255; 4],
        })
    };
    let controller = presentation.item_icon("test:controller", 0).unwrap();
    let gem = presentation.item_icon("test:gem", 0).unwrap();
    let mut nodes = vec![slot(controller), slot(gem)];
    presentation.apply_gui_models(&mut nodes);
    let UiVisual::Mesh(mesh) = nodes[0].visual() else {
        panic!("a custom cube block item must draw GUI geometry");
    };
    let inside = |[u, v]: [f32; 2], uv: [u16; 4]| {
        u >= f32::from(uv[0])
            && u <= f32::from(uv[2])
            && v >= f32::from(uv[1])
            && v <= f32::from(uv[3])
    };
    assert!(
        mesh.vertices()
            .iter()
            .all(|vertex| !inside(vertex.uv, controller.uv)),
        "the cube samples the sheet, not the thumbnail"
    );
    assert!(matches!(nodes[1].visual(), UiVisual::Sprite { .. }));
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
