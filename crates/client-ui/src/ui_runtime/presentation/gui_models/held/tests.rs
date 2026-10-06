use super::*;

#[test]
fn held_block_is_the_shared_six_face_cube_not_a_gui_thumbnail() {
    let source = IconRef {
        page: 7,
        uv: [
            10,
            20,
            10 + assets::BLOCK_ITEM_SHEET_SIZE[0],
            20 + assets::BLOCK_ITEM_SHEET_SIZE[1],
        ],
        glint: false,
    };
    let model = block(source, [[0.0; 3]; 2]);
    assert_eq!(model.source, source);
    assert_eq!(model.vertices.len(), 36);
    for axis in 0..3 {
        assert!(
            model
                .vertices
                .iter()
                .any(|vertex| vertex.position[axis] == -0.5)
        );
        assert!(
            model
                .vertices
                .iter()
                .any(|vertex| vertex.position[axis] == 0.5)
        );
    }
    assert!(
        model
            .placements
            .iter()
            .all(|placement| matches!(placement, PreviewHeldPlacement::Block))
    );
    for (face, vertices) in model.vertices.chunks_exact(6).enumerate() {
        let icon = super::super::sheet_faces(source)[face];
        assert_eq!(
            vertices[0].uv,
            [
                f32::from(icon.uv[0] - source.uv[0]) / f32::from(assets::BLOCK_ITEM_SHEET_SIZE[0]),
                f32::from(icon.uv[1] - source.uv[1]) / f32::from(assets::BLOCK_ITEM_SHEET_SIZE[1]),
            ]
        );
    }
}

/// Compiles original item bones and retains entity-only banner routes without cube sheets.
fn banner_catalog() -> (RuntimeEntityAssets, RuntimeIconCatalog) {
    let entity = br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","geometry":{"default":"geometry.test_player"},"render_controllers":["controller.render.test_player"]}}}"#;
    let geometry = br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.test_player","texture_width":64,"texture_height":64},"bones":[{"name":"body","pivot":[0,0,0],"cubes":[{"origin":[-4,0,-2],"size":[8,12,4],"uv":[0,0]}]},{"name":"rightItem","parent":"body","pivot":[-6,12,0]},{"name":"leftItem","parent":"body","pivot":[6,12,0]}]}]}"#;
    let render = br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test_player":{"geometry":"Geometry.default"}}}"#;
    let mut compiled = pack_compiler::compile_entity_pack(vec![
        ("entity/player.json".into(), entity.to_vec()),
        ("models/entity/player.geo.json".into(), geometry.to_vec()),
        ("render_controllers/player.json".into(), render.to_vec()),
    ])
    .unwrap()
    .unwrap()
    .assets;
    compiled.block_visual_count = 2;
    compiled.item_visuals = (0..16)
        .map(|metadata| assets::ItemVisualDefinition {
            key: ItemVisualKey {
                identifier: "minecraft:banner".into(),
                metadata,
            },
            source: 0,
            route: ItemVisualDefinitionRoute::BlockItem {
                block_visual: assets::BlockVisualId(1),
            },
            first_person: assets::ItemDisplayTransform::identity(),
            third_person: assets::ItemDisplayTransform::identity(),
            dropped: assets::ItemDisplayTransform::identity(),
        })
        .collect();
    let entities = RuntimeEntityAssets::from_compiled(compiled).unwrap();
    let sprites = (0..16)
        .map(|metadata| {
            let color = assets::banner::color_rgb(metadata);
            let rgba8 = (0..16 * 16)
                .flat_map(|pixel| {
                    let [x, y] = [pixel % 16, pixel / 16];
                    [
                        color[0],
                        color[1],
                        color[2],
                        if (5..=10).contains(&x) && y < 15 {
                            255
                        } else {
                            0
                        },
                    ]
                })
                .collect::<Vec<_>>();
            assets::IconSprite {
                width: 16,
                height: 16,
                rgba8: rgba8.into(),
            }
        })
        .collect::<Vec<_>>();
    let entries = (0..16)
        .map(|metadata| assets::IconEntry {
            identifier: "minecraft:banner".into(),
            metadata,
            sprite: metadata,
        })
        .collect::<Vec<_>>();
    let icons = RuntimeIconCatalog::decode(
        &assets::encode_icon_catalog(entities.source_manifest_sha256(), &sprites, &entries)
            .unwrap(),
    )
    .unwrap();
    (entities, icons)
}

#[test]
fn held_banner_block_route_without_cube_sheet_remains_available_in_both_preview_hands() {
    let (entities, icons) = banner_catalog();
    assert!(icons.block_sheets().is_empty());
    let mut atlas = atlas::Atlas::new(1, super::super::MODEL_PAGES);
    let models = prepare(&mut atlas, &entities, &icons, None, &BTreeMap::new()).unwrap();
    assert_eq!(models.len(), icons.entries().len());
    let skin = IconRef {
        page: 0,
        uv: [0, 0, 64, 64],
        glint: false,
    };
    let draw = |hands| {
        player_preview::geometry::mesh(
            Default::default(),
            Default::default(),
            0.0,
            skin,
            &Default::default(),
            [None; 4],
            hands,
            true,
        )
        .unwrap()
    };
    let bare = draw([None; 2]).vertices().len();
    for definition in entities.item_visuals() {
        assert!(matches!(
            definition.route,
            ItemVisualDefinitionRoute::BlockItem { .. }
        ));
        let model = models
            .get(&definition.key)
            .expect("noncube block item retains its held model");
        assert!(
            model
                .placements
                .iter()
                .all(|placement| matches!(placement, PreviewHeldPlacement::Sprite { .. }))
        );
        let right = draw([Some(model), None]).vertices().len();
        let left = draw([None, Some(model)]).vertices().len();
        assert!(right > bare && left > bare);
        assert_eq!(draw([Some(model); 2]).vertices().len(), right + left - bare);
    }
}
