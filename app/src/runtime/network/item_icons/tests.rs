use std::{io::Write, sync::Arc};

use resource_pack::LayeredPackView;

use super::{BlockIcons, compile_session_icons, custom_block_items};
use crate::ui_runtime::presentation::SessionIcon;

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |_, y| image::Rgba([y as u8, 0, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn view() -> LayeredPackView {
    let id = "00000000-0000-0000-0000-000000000002";
    let manifest = format!(
        r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
    );
    let catalog = r#"{"texture_data": {"test:gem": {"textures": "textures/items/gem"},
        "test:strip": {"textures": ["textures/items/strip"]},
        "test:huge": {"textures": "textures/items/huge"}}}"#;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, bytes) in [
        ("manifest.json", manifest.into_bytes()),
        ("textures/item_texture.json", catalog.as_bytes().to_vec()),
        ("textures/items/gem.png", png(16, 16)),
        ("textures/items/strip.png", png(16, 48)),
        ("textures/items/huge.png", png(128, 64)),
    ] {
        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
    }
    let archive = protocol::ResourcePackArchive::unencrypted(
        id.parse().unwrap(),
        "1.0.0".into(),
        String::new(),
        writer.finish().unwrap().into_inner(),
    );
    LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(vec![archive]),
    ))
}

// Keys resolve through item_texture.json; strips keep frame one and big icons shrink.
#[test]
fn icon_keys_resolve_to_bounded_sprites() {
    let key = |identifier: &str, key: &str| (Arc::<str>::from(identifier), Arc::<str>::from(key));
    let icons = compile_session_icons(
        &view(),
        &[
            key("lifeboat:gem", "test:gem"),
            key("lifeboat:strip", "test:strip"),
            key("lifeboat:huge", "test:huge"),
            key("lifeboat:missing", "test:absent"),
        ],
        BlockIcons::default(),
    )
    .expect("icons");
    let sizes = icons
        .icons
        .iter()
        .map(|icon| (icon.identifier.as_ref(), icon.width, icon.height))
        .collect::<Vec<_>>();
    assert_eq!(
        sizes,
        [
            ("lifeboat:gem", 16, 16),
            ("lifeboat:strip", 16, 16),
            ("lifeboat:huge", 64, 32)
        ]
    );
    let strip = &icons.icons[1];
    assert_eq!(strip.rgba8[(15 * 16) * 4], 15, "first frame rows only");
}

fn stack(packs: &[&[(&str, Vec<u8>)]]) -> LayeredPackView {
    let archives = packs
        .iter()
        .enumerate()
        .map(|(index, files)| {
            let id = format!("00000000-0000-0000-0000-{index:012}");
            let manifest = format!(
                r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
            );
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (path, bytes) in std::iter::once(("manifest.json", manifest.into_bytes()))
                .chain(files.iter().map(|(path, bytes)| (*path, bytes.clone())))
            {
                writer
                    .start_file(path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(&bytes).unwrap();
            }
            protocol::ResourcePackArchive::unencrypted(
                id.parse().unwrap(),
                "1.0.0".into(),
                String::new(),
                writer.finish().unwrap().into_inner(),
            )
        })
        .collect();
    LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(archives),
    ))
}

// Keys merge across every pack, fall back to textures/items, and misses say why.
#[test]
fn custom_item_icons_merge_across_packs_and_explain_misses() {
    let key = |identifier: &str, key: &str| (Arc::<str>::from(identifier), Arc::<str>::from(key));
    let base_catalog = br#"{"texture_data":{"a_key":{"textures":"textures/items/a"}}}"#.to_vec();
    let top_catalog = br#"{"texture_data":{"b_key":{"textures":"textures/items/b"},"dead_key":{"textures":"textures/items/none"}}}"#.to_vec();
    let view = stack(&[
        &[
            ("textures/item_texture.json", base_catalog),
            ("textures/items/a.png", png(16, 16)),
        ],
        &[
            ("textures/item_texture.json", top_catalog),
            ("textures/items/b.png", png(16, 16)),
            ("textures/items/loose.png", png(16, 16)),
        ],
    ]);
    let icons = compile_session_icons(
        &view,
        &[
            key("t:a", "a_key"),
            key("t:b", "b_key"),
            key("t:loose", "loose"),
            key("t:dead", "dead_key"),
            key("t:absent", "missing_key"),
        ],
        BlockIcons::default(),
    )
    .expect("icons");
    let mut resolved: Vec<_> = icons
        .icons
        .iter()
        .map(|i| i.identifier.to_string())
        .collect();
    resolved.sort();
    assert_eq!(resolved, ["t:a", "t:b", "t:loose"]);
    assert!(icons.misses["t:dead"].contains("no readable image"));
    assert!(icons.misses["t:absent"].contains("not in the merged item_texture.json"));
}

// A catalog path that already names its image resolves, as on the retail client.
#[test]
fn catalog_paths_with_an_image_extension_resolve() {
    let catalog = br#"{"texture_data":{"zeqa.training":{"textures":"textures/items/zeqa/hub/main/training.png"},"upper":{"textures":"textures/items/upper.PNG"}}}"#.to_vec();
    let view = stack(&[&[
        ("textures/item_texture.json", catalog),
        ("textures/items/zeqa/hub/main/training.png", png(16, 16)),
        ("textures/items/upper.PNG", png(16, 16)),
    ]]);
    let key = |identifier: &str, key: &str| (Arc::<str>::from(identifier), Arc::<str>::from(key));
    let icons = compile_session_icons(
        &view,
        &[
            key("zeqa:item.training", "zeqa.training"),
            key("t:upper", "upper"),
        ],
        BlockIcons::default(),
    )
    .expect("icons");
    assert!(icons.misses.is_empty(), "{:?}", icons.misses);
    assert_eq!(icons.icons.len(), 2);
}

// A custom block item draws as its block even when a short-name guess would resolve.
#[test]
fn block_items_beat_short_name_guesses() {
    let key = |identifier: &str, key: &str| (Arc::<str>::from(identifier), Arc::<str>::from(key));
    let block = SessionIcon {
        identifier: "t:crate".into(),
        width: 32,
        height: 32,
        rgba8: vec![7; 32 * 32 * 4].into(),
    };
    let blocks = BlockIcons {
        icons: vec![block],
        misses: vec![("t:broken".into(), "no drawable visual".into())],
    };
    let icons = compile_session_icons(
        &view(),
        &[
            key("t:crate", "test:gem"),
            key("t:broken", "test:gem"),
            key("t:gem", "test:gem"),
        ],
        blocks,
    )
    .expect("icons");
    let crate_icon = icons
        .icons
        .iter()
        .find(|icon| icon.identifier.as_ref() == "t:crate")
        .expect("block icon");
    assert_eq!((crate_icon.width, crate_icon.rgba8[0]), (32, 7));
    assert_eq!(icons.icons.len(), 2, "t:broken keeps no sprite");
    assert!(icons.misses["t:broken"].contains("no drawable visual"));
}

// A registry item named after a custom block is that block's item; others are not.
#[test]
fn registry_items_named_after_custom_blocks_are_block_items() {
    let mut game_data = protocol::GameData {
        start_game: Default::default(),
        item_registry: Default::default(),
        biome_definitions: None,
        entity_identifiers: None,
        creative_content: None,
    };
    for name in ["t:crate", "t:sword"] {
        game_data.item_registry.item_data.push(Default::default());
        game_data
            .item_registry
            .item_data
            .last_mut()
            .unwrap()
            .item_name = name.into();
    }
    let blocks = protocol::CustomBlocks {
        blocks: vec![protocol::CustomBlock {
            name: "t:crate".into(),
            state_count: 1,
            collides: true,
            collision_box: None,
            selection: Default::default(),
            visual: Default::default(),
        }]
        .into(),
        skipped: 0,
    };
    let pairs = custom_block_items(&game_data, &blocks);
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        (pairs[0].0.as_ref(), pairs[0].1.as_ref()),
        ("t:crate", "t:crate")
    );
}

// Real cached packs (`CINNABAR_PACKCACHE_DIR`): every item_texture.json key a pack declares
// resolves to a bounded icon through the same path a `minecraft:icon` component takes.
#[test]
fn packcache_item_icon_keys_resolve_when_requested() {
    let Some(dir) = std::env::var_os("CINNABAR_PACKCACHE_DIR") else {
        return;
    };
    let (mut declared, mut resolved) = (0usize, 0usize);
    for entry in std::fs::read_dir(dir).expect("packcache dir").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "zip") {
            continue;
        }
        let Some(view) = super::super::local_pack::local_pack_view_at(&path) else {
            continue;
        };
        let keys =
            super::super::resource_packs::texture_key_paths(&view, "textures/item_texture.json")
                .into_keys()
                .map(|key| {
                    (
                        Arc::<str>::from(format!("pack:{key}")),
                        Arc::<str>::from(key),
                    )
                })
                .collect::<Vec<_>>();
        for chunk in keys.chunks(256) {
            declared += chunk.len();
            resolved += compile_session_icons(&view, chunk, BlockIcons::default())
                .map_or(0, |icons| icons.icons.len());
        }
    }
    eprintln!("{resolved} of {declared} cached-pack item icon keys resolved");
    assert!(resolved * 10 >= declared * 8, "{resolved} of {declared}");
}
