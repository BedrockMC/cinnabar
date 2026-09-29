use std::io::Write;

use resource_pack::LayeredPackView;

use super::compile_session_glyphs;

fn sheet_png(cell: u32, opaque_cell: u32) -> Vec<u8> {
    let side = cell * 16;
    let image = image::RgbaImage::from_fn(side, side, |x, y| {
        let index = (y / cell) * 16 + x / cell;
        image::Rgba([255, 255, 255, if index == opaque_cell { 255 } else { 0 }])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn archive(id: u8, files: &[(&str, Vec<u8>)]) -> protocol::ResourcePackArchive {
    let id = format!("00000000-0000-0000-0000-{id:012}");
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
}

fn view(archives: Vec<protocol::ResourcePackArchive>) -> LayeredPackView {
    LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(archives),
    ))
}

#[test]
fn a_stack_without_glyph_sheets_yields_none() {
    assert!(compile_session_glyphs(&view(vec![archive(1, &[])])).is_none());
}

// The last-applied pack replaces a sheet wholesale; sheets it lacks still come from lower packs.
#[test]
fn the_last_applied_pack_wins_per_sheet() {
    let sheets = compile_session_glyphs(&view(vec![
        archive(
            1,
            &[
                ("font/glyph_E0.png", sheet_png(8, 1)),
                ("font/glyph_E1.png", sheet_png(8, 2)),
            ],
        ),
        archive(2, &[("font/glyph_E0.png", sheet_png(16, 3))]),
    ]))
    .expect("sheets");
    let cell = |c: char| {
        sheets
            .cells
            .iter()
            .find(|cell| cell.codepoint == c)
            .unwrap()
    };
    assert_eq!(sheets.cells.len(), 512);
    // E0 comes from the top pack (16 px cells, cell 3 opaque); E1 falls through to the lower one.
    assert_eq!(cell('\u{e003}').size, [16, 16]);
    assert_eq!(cell('\u{e002}').size, [0, 0]);
    assert_eq!(cell('\u{e102}').size, [8, 8]);
}

// Local-only: set CINNABAR_SERVER_PACK to a cached server `.mcpack` to check its sheets decode and pack.
#[test]
fn a_real_server_pack_decodes_and_packs() {
    let Some(view) = crate::runtime::network::local_pack::local_pack_view("CINNABAR_SERVER_PACK")
    else {
        return;
    };
    let Some(sheets) = compile_session_glyphs(&view) else {
        eprintln!("no glyph sheets");
        return;
    };
    let atlas = assets::pack_cells(&sheets.cells, 0, 256, 8);
    assert!(!atlas.glyphs.is_empty());
    eprintln!(
        "{} cells, {} glyphs, {} pages",
        sheets.cells.len(),
        atlas.glyphs.len(),
        atlas.pages.len()
    );
}
