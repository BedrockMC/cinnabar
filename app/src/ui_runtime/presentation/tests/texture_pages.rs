use super::*;

fn independent_font(sides: &[u32]) -> Arc<RuntimeFontCatalog> {
    let pages = sides
        .iter()
        .enumerate()
        .map(|(index, &side)| {
            let pixels = vec![255; side as usize * side as usize * 4].into_boxed_slice();
            FontTexturePage {
                source_path: format!("font/independent-{index:02}.png").into(),
                source_bytes: pixels.len() as u32,
                source_sha256: Sha256::digest(&pixels).into(),
                pixels_sha256: Sha256::digest(&pixels).into(),
                width: side,
                height: side,
                rgba8: pixels,
            }
        })
        .collect::<Vec<_>>();
    let glyph = GlyphMetrics {
        codepoint: '\u{fffd}',
        page: 0,
        uv: [0, 0, 8, 16],
        bearing: [0, -14],
        advance_64: 8 * 64,
    };
    let mut glyphs = vec![
        GlyphMetrics {
            codepoint: 'A',
            ..glyph
        },
        glyph,
    ];
    if sides.len() > 1 {
        glyphs.push(GlyphMetrics {
            codepoint: '一',
            page: 1,
            ..glyph
        });
    }
    glyphs.sort_by_key(|g| g.codepoint);
    let bytes = encode_font_catalog([7; 32], &glyphs, &pages).unwrap();
    Arc::new(RuntimeFontCatalog::decode(&bytes, [7; 32]).unwrap())
}

fn independent_icons(count: usize, side: u16) -> Arc<RuntimeIconCatalog> {
    let sprite = assets::IconSprite {
        width: side,
        height: side,
        rgba8: vec![255; usize::from(side).pow(2) * 4].into(),
    };
    let sprites = vec![sprite; count];
    let entries = (0..count)
        .map(|i| assets::IconEntry {
            identifier: format!("minecraft:fixture_{i:04}").into(),
            metadata: 0,
            sprite: i as u32,
        })
        .collect::<Vec<_>>();
    Arc::new(
        RuntimeIconCatalog::decode(
            &assets::encode_icon_catalog([5; 32], &sprites, &entries).unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn full_icon_catalog_and_reserved_dynamic_pages_are_admitted_together() {
    let font = independent_font(&[1024, 2048, 2048, 2048]);
    let presentation = UiPresentationRuntime::with_hud_and_icons(
        Arc::clone(&font),
        fixture_hud(),
        independent_icons(735, 16),
    )
    .unwrap();
    assert_eq!(
        presentation.textures.plan().bytes(),
        55 * 1024 * 1024 + 768 * 1024
    );
    assert_eq!(presentation.icon_refs.as_ref().unwrap().len(), 735);
    assert!(
        UiPresentationRuntime::with_hud_and_icons(font, fixture_hud(), independent_icons(900, 64))
            .is_err(),
        "whole valid large icon catalog must refuse, not truncate to fit"
    );
}

#[test]
fn mixed_font_shadow_and_fill_keep_logical_page_order() {
    let mut presentation = UiPresentationRuntime::new(independent_font(&[1024, 2048])).unwrap();
    let mut runtime = UiRuntime::new(1);
    runtime.open_chat();
    runtime.insert_chat_text("A一A").unwrap();
    let input = presentation
        .build(&runtime, 0, [800, 600], DpiScale::new(1.0).unwrap())
        .unwrap();
    let pages = input
        .batches
        .iter()
        .filter(|b| b.texture_page < 2)
        .map(|b| b.texture_page)
        .collect::<Vec<_>>();
    assert!(
        pages.windows(5).any(|w| w == [0, 1, 0, 1, 0]),
        "shadow then fill retain alternating font pages, even if adjacent page0 spans merge: {pages:?}"
    );
    for batch in input.batches.iter() {
        let logical = batch.texture_page as usize;
        let physical = input.textures.plan().locations()[logical];
        assert_eq!(
            input.textures.plan().buckets()[physical.bucket].dimensions,
            input.textures.pages()[logical].dimensions()
        );
    }
}

#[test]
fn mixed_native_font_pages_fit_ui_without_max_side_padding() {
    let font = independent_font(&[1024, 2048, 2048, 2048]);
    let presentation = UiPresentationRuntime::new(Arc::clone(&font)).unwrap();
    assert_eq!(
        presentation.textures.plan().bytes(),
        52 * 1024 * 1024 + 10 * 256 * 256 * 4
    );
    for (index, source) in font.pages().iter().enumerate() {
        let page = &presentation.textures.pages()[index];
        assert_eq!(page.dimensions(), [source.width, source.height]);
        assert_eq!(page.pixels().as_ptr(), source.rgba8.as_ptr());
    }
}

#[test]
fn actual_preview_updates_share_static_pages_and_retire_superseded_scenes() {
    let mut presentation = UiPresentationRuntime::new(independent_font(&[256])).unwrap();
    presentation.set_player_preview_skin(None, player_preview::PlayerPreviewPose::default());
    let retained = Arc::clone(&presentation.textures);
    let static_pointer = retained.pages()[0].pixels().as_ptr();
    let retained_dynamic = retained.pages()[retained.dynamic_start()].pixels().to_vec();
    let stats = UiRenderStats::default();
    let mut main = UiRenderScene::default();
    let mut extracted = UiRenderScene::default();
    assert!(extracted.input.is_none());
    let runtime = UiRuntime::new(1);
    let mut prior = None;
    for frame in 1..=100 {
        presentation.set_player_preview_skin(
            None,
            player_preview::PlayerPreviewPose::new(frame as f32, 40.0, 10.0, false),
        );
        assert_eq!(
            presentation.textures.pages()[0].pixels().as_ptr(),
            static_pointer
        );
        let input = presentation
            .build(&runtime, 0, [800, 600], DpiScale::new(1.0).unwrap())
            .unwrap();
        main.publish(input, &stats).unwrap();
        // Same single-target replacement used by Bevy's ExtractResource.
        extracted = main.clone();
        if let Some(old) = prior.take() {
            assert!(std::sync::Weak::upgrade(&old).is_none());
        }
        prior = Some(Arc::downgrade(&presentation.textures));
        for index in 0..presentation.textures.dynamic_start() {
            assert!(std::ptr::eq(
                presentation.textures.pages()[index].pixels(),
                retained.pages()[index].pixels()
            ));
        }
    }
    assert_ne!(presentation.textures.identity(), retained.identity());
    assert_eq!(
        retained.pages()[retained.dynamic_start()].pixels(),
        retained_dynamic
    );
    let before = Arc::clone(&presentation.textures);
    presentation.set_player_preview_skin(
        None,
        player_preview::PlayerPreviewPose::new(100.0, 40.0, 10.0, false),
    );
    assert!(Arc::ptr_eq(&before, &presentation.textures));
    assert!(extracted.input.is_some());
}

#[test]
fn resize_and_session_reset_do_not_reload_static_pixels_or_retain_dynamic_ownership() {
    let mut presentation = UiPresentationRuntime::new(independent_font(&[256])).unwrap();
    let runtime = UiRuntime::new(1);
    presentation.set_player_preview_skin(None, player_preview::PlayerPreviewPose::default());
    let first = presentation
        .build(&runtime, 0, [800, 600], DpiScale::new(1.0).unwrap())
        .unwrap();
    let static_pixel = first.textures.pages()[0].pixels().as_ptr();
    let resized = presentation
        .build(&runtime, 0, [1200, 800], DpiScale::new(1.0).unwrap())
        .unwrap();
    assert!(Arc::ptr_eq(&first.textures, &resized.textures));
    assert!(resized.revision > first.revision);
    let reset = presentation
        .build(
            &UiRuntime::new(2),
            0,
            [1200, 800],
            DpiScale::new(1.0).unwrap(),
        )
        .unwrap();
    assert_eq!(reset.textures.pages()[0].pixels().as_ptr(), static_pixel);
    assert_eq!(
        reset.textures.static_identity(),
        first.textures.static_identity()
    );
    assert!(
        reset.textures.pages()[reset.textures.dynamic_start()..]
            .iter()
            .all(|p| p.pixels().iter().all(|&v| v == 0))
    );
    assert!(presentation.player_preview_icon.is_none());
    assert!(reset.revision > resized.revision);
    let stats = UiRenderStats::default();
    let mut scene = UiRenderScene::default();
    scene.publish(reset, &stats).unwrap();
    assert!(scene.publish(first, &stats).is_err());
    assert!(scene.input.is_none());
    assert_eq!(stats.snapshot().accepted_revision, None);
}

#[test]
fn preview_changes_do_not_reread_menu_files_or_copy_cached_menu_pages() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "ui-independent-art-{}-{unique}.png",
        std::process::id()
    ));
    image::RgbaImage::from_pixel(96, 96, image::Rgba([40, 80, 120, 255]))
        .save(&path)
        .unwrap();
    let mut presentation = UiPresentationRuntime::new(independent_font(&[256])).unwrap();
    let name = path.to_string_lossy().into_owned();
    presentation.sync_menu_artwork(vec![name.clone()]);
    let menu = presentation.menu_artwork_icon(&name).unwrap();
    let retained = Arc::clone(&presentation.textures);
    let menu_pixels = retained.pages()[menu.page as usize].pixels().as_ptr();
    std::fs::remove_file(&path).unwrap();
    for frame in 0..100 {
        presentation.set_player_preview_skin(
            None,
            player_preview::PlayerPreviewPose::new(frame as f32, 20.0, 0.0, false),
        );
        assert_eq!(presentation.menu_artwork_icon(&name), Some(menu));
        assert_eq!(
            presentation.textures.pages()[menu.page as usize]
                .pixels()
                .as_ptr(),
            menu_pixels
        );
        assert!(
            presentation.menu_artwork.pages.is_empty(),
            "no second raster cache owner"
        );
    }
    let preview_pixels = presentation.textures.pages()[presentation.textures.dynamic_start()]
        .pixels()
        .as_ptr();
    presentation.sync_menu_artwork(Vec::new());
    assert!(presentation.menu_artwork_icon(&name).is_none());
    assert_eq!(
        presentation.textures.pages()[presentation.textures.dynamic_start()]
            .pixels()
            .as_ptr(),
        preview_pixels
    );
}
