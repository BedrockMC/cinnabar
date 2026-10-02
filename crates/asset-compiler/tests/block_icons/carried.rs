use super::*;

fn grass_pack(side_entry: serde_json::Value) -> tempfile::TempDir {
    let pack = pack();
    write(
        pack.path(),
        "blocks.json",
        br#"{"grass":{"textures":{"up":"world_top","down":"bottom","side":"world_side"},"carried_textures":{"up":"carried_top","down":"bottom","side":"carried_side"}}}"#,
    );
    write(
        pack.path(),
        "textures/terrain_texture.json",
        &serde_json::to_vec(&serde_json::json!({"texture_data":{
            "world_top":{"textures":"textures/blocks/top"},
            "world_side":{"textures":"textures/blocks/side"},
            "bottom":{"textures":"textures/blocks/bottom"},
            "carried_top":{"textures":"textures/blocks/top"},
            "carried_side":{"textures":side_entry}
        }}))
        .unwrap(),
    );
    write(pack.path(), "textures/flipbook_textures.json", b"[]");
    fs::create_dir_all(pack.path().join("textures/blocks")).unwrap();
    for (path, pixel) in [("top", [30, 140, 20, 255]), ("bottom", [110, 70, 35, 255])] {
        image::save_buffer(
            pack.path().join(format!("textures/blocks/{path}.png")),
            &pixel.repeat((TILE_SIZE as usize).pow(2)),
            TILE_SIZE,
            TILE_SIZE,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    // Bedrock's opaque overlay material treats alpha as a tint mask, not
    // opacity: alpha-zero soil keeps its RGB; alpha-one grey foliage is tinted.
    let mut side = Vec::new();
    for y in 0..TILE_SIZE {
        side.extend_from_slice(
            &if y < TILE_SIZE / 2 {
                [200, 200, 200, 255]
            } else {
                [110, 70, 35, 0]
            }
            .repeat(TILE_SIZE as usize),
        );
    }
    image::save_buffer(
        pack.path().join("textures/blocks/side.png"),
        &side,
        TILE_SIZE,
        TILE_SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();
    pack
}

fn grass_world(entity: &CompiledEntityAssets) -> (CompiledAssets, BlockVisualId) {
    let mut source = world(entity);
    let ItemVisualDefinitionRoute::BlockItem { block_visual } = entity
        .item_visuals
        .iter()
        .find(|visual| visual.key.identifier.as_ref() == "minecraft:grass_block")
        .expect("pinned registry grass item")
        .route
    else {
        panic!("grass requires a block item route");
    };
    source.visuals[block_visual.0 as usize] = source
        .visuals
        .iter()
        .copied()
        .find(|visual| visual.kind == VisualKind::Cube)
        .unwrap();
    source.materials[1].flags = MATERIAL_FLAG_GRASS_TINT | MATERIAL_FLAG_OVERLAY_MASK;
    (source, block_visual)
}

#[test]
fn carried_overlay_grass_generates_a_visible_inventory_icon() {
    let pack = grass_pack(serde_json::json!({
        "path":"textures/blocks/side", "overlay_color":"#79c05a"
    }));
    let entity = compile_entity_assets(pack.path(), MANIFEST).unwrap();
    let (source, _) = grass_world(&entity);
    let compiled =
        compile_icon_assets_with_blocks(pack.path(), MANIFEST, &runtime(&source)).unwrap();
    let catalog = RuntimeIconCatalog::decode(&compiled.bytes).unwrap();
    let sprite = catalog
        .lookup("minecraft:grass_block", 0)
        .expect("carried overlay grass must not disappear");
    assert!(sprite.rgba8.chunks_exact(4).any(|pixel| pixel[3] == 255));
}

fn sheet_pixel(sheet: &IconSprite, face: BlockFace, x: usize, y: usize) -> &[u8] {
    let side = usize::from(BLOCK_ITEM_FACE_SIDE);
    let columns = usize::from(BLOCK_ITEM_SHEET_GRID[0]);
    let tile = face as usize;
    let x = (tile % columns) * side + x;
    let y = (tile / columns) * side + y;
    let offset = (y * usize::from(sheet.width) + x) * 4;
    &sheet.rgba8[offset..offset + 4]
}

#[test]
fn carried_overlay_sheet_masks_tint_without_coloring_or_hiding_soil() {
    let pack = grass_pack(serde_json::json!({
        "path":"textures/blocks/side", "overlay_color":"#79c05a"
    }));
    let entity = compile_entity_assets(pack.path(), MANIFEST).unwrap();
    let (source, visual) = grass_world(&entity);
    let compiled =
        compile_icon_assets_with_blocks(pack.path(), MANIFEST, &runtime(&source)).unwrap();
    let catalog = RuntimeIconCatalog::decode(&compiled.bytes).unwrap();
    let binding = catalog
        .block_sheets()
        .iter()
        .find(|binding| binding.visual == visual)
        .expect("the grass item publishes its carried cube sheet");
    let sheet = &catalog.sprites()[binding.sprite as usize];
    assert_eq!([sheet.width, sheet.height], BLOCK_ITEM_SHEET_SIZE);
    for face in [
        BlockFace::West,
        BlockFace::East,
        BlockFace::North,
        BlockFace::South,
    ] {
        assert_eq!(sheet_pixel(sheet, face, 0, 0), &[94, 150, 70, 255]);
        assert_eq!(
            sheet_pixel(sheet, face, 0, usize::from(BLOCK_ITEM_FACE_SIDE) - 1),
            &[110, 70, 35, 255]
        );
    }
    assert_eq!(
        sheet_pixel(sheet, BlockFace::Down, 0, 0),
        &[110, 70, 35, 255]
    );
    assert_eq!(sheet_pixel(sheet, BlockFace::Up, 0, 0), &[30, 140, 20, 255]);
    assert!(sheet.rgba8.chunks_exact(4).all(|pixel| pixel[3] == 255));
    assert_eq!(
        compiled.bytes,
        encode_icon_catalog_with_block_sheets(
            catalog.source_manifest_sha256(),
            catalog.sprites(),
            catalog.entries(),
            catalog.block_sheets()
        )
        .unwrap()
    );
}

#[test]
fn carried_overlay_ignores_hex_high_alpha_byte_and_keeps_partial_mask() {
    let pack = grass_pack(serde_json::json!({
        "path":"textures/blocks/side", "overlay_color":"#0079c05a"
    }));
    let pixels = [200, 200, 200, 128].repeat((TILE_SIZE as usize).pow(2));
    image::save_buffer(
        pack.path().join("textures/blocks/side.png"),
        &pixels,
        TILE_SIZE,
        TILE_SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();
    let entity = compile_entity_assets(pack.path(), MANIFEST).unwrap();
    let (source, visual) = grass_world(&entity);
    let compiled =
        compile_icon_assets_with_blocks(pack.path(), MANIFEST, &runtime(&source)).unwrap();
    let catalog = RuntimeIconCatalog::decode(&compiled.bytes).unwrap();
    let binding = catalog
        .block_sheets()
        .iter()
        .find(|sheet| sheet.visual == visual)
        .unwrap();
    let sheet = &catalog.sprites()[binding.sprite as usize];
    assert_eq!(
        sheet_pixel(sheet, BlockFace::West, 0, 0),
        &[147, 175, 135, 255]
    );
}

#[test]
fn carried_overlay_unknown_metadata_and_malformed_colors_remain_unresolved() {
    for side in [
        serde_json::json!({"path":"textures/blocks/side", "overlay_color":"green"}),
        serde_json::json!({"path":"textures/blocks/side", "overlay_color":"#abcd"}),
        serde_json::json!({"path":"textures/blocks/side", "overlay_color":"#79c05z"}),
        serde_json::json!({"path":"textures/blocks/side", "overlay_color":"#79c05a", "unknown":true}),
    ] {
        let pack = grass_pack(side);
        let entity = compile_entity_assets(pack.path(), MANIFEST).unwrap();
        let (source, visual) = grass_world(&entity);
        let compiled =
            compile_icon_assets_with_blocks(pack.path(), MANIFEST, &runtime(&source)).unwrap();
        let catalog = RuntimeIconCatalog::decode(&compiled.bytes).unwrap();
        assert!(catalog.lookup("minecraft:grass_block", 0).is_none());
        assert!(
            !catalog
                .block_sheets()
                .iter()
                .any(|sheet| sheet.visual == visual)
        );
    }
}

#[test]
fn carried_overlay_does_not_grant_held_cube_geometry_to_nonopaque_shapes() {
    let pack = grass_pack(serde_json::json!({
        "path":"textures/blocks/side", "overlay_color":"#79c05a"
    }));
    let entity = compile_entity_assets(pack.path(), MANIFEST).unwrap();
    let (mut source, visual) = grass_world(&entity);
    source.visuals[visual.0 as usize].flags = BlockFlags::CUBE_GEOMETRY;
    let compiled =
        compile_icon_assets_with_blocks(pack.path(), MANIFEST, &runtime(&source)).unwrap();
    let catalog = RuntimeIconCatalog::decode(&compiled.bytes).unwrap();
    assert!(
        !catalog
            .block_sheets()
            .iter()
            .any(|sheet| sheet.visual == visual)
    );
}
