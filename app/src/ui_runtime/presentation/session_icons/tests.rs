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
fn large_session_icons_install_without_blocking_later_server_ui_textures() {
    use super::super::forms::{ServerUiPack, pack_harness, tests::mini_engine_presentation};

    let mut presentation = mini_engine_presentation();
    let static_identity = presentation.textures.static_identity();
    let icons = Arc::new(SessionIcons {
        icons: (0..600).map(sprite).collect(),
        misses: HashMap::new(),
    });
    observe(&mut presentation, Some(&icons));
    let icon = presentation.item_icon("test:variant", 599).unwrap();
    let page = &presentation.textures.pages()[usize::from(icon.page)];
    assert_eq!(
        page.dimensions(),
        [MIN_PAGE_SIDE * 2; 2],
        "session icon references must address the installed enlarged page"
    );
    let pixel =
        ((u32::from(icon.uv[1]) * page.dimensions()[0] + u32::from(icon.uv[0])) * 4) as usize;
    assert_eq!(&page.pixels()[pixel..pixel + 4], &[599u32 as u8; 4]);
    assert_eq!(presentation.textures.static_identity(), static_identity);

    let color = [17, 91, 203, 255];
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(8, 8, image::Rgba(color))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let path = "textures/ui/session_icon_upload_witness";
    presentation.set_server_ui_pack(&ServerUiPack {
        textures: vec![(format!("{path}.png"), png.into_inner())],
        ..Default::default()
    });
    let runtime = pack_harness::image_form(
        "Image",
        &["Image"],
        vec![Some(protocol::FormButtonImage::Path(path.into()))],
    );
    let nodes = pack_harness::render(&mut presentation, &runtime, [1280, 720], 1.0);
    let server_page = presentation.textures.dynamic_start() + dynamic_textures::SERVER_UI_PAGE;
    let uv = nodes
        .iter()
        .find_map(|node| match node.visual() {
            ui::UiVisual::Sprite {
                texture_page, uv, ..
            } if usize::from(*texture_page) == server_page => Some(*uv),
            _ => None,
        })
        .expect("the form draws the server texture");
    let page = &presentation.textures.pages()[server_page];
    let pixel = ((u32::from(uv[1]) * page.dimensions()[0] + u32::from(uv[0])) * 4) as usize;
    assert_eq!(
        &page.pixels()[pixel..pixel + 4],
        &color,
        "a large session icon page must not block later server UI uploads"
    );
    assert_eq!(presentation.textures.static_identity(), static_identity);
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
