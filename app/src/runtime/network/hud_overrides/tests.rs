use std::io::Write;

use resource_pack::LayeredPackView;

use super::compile_hud_overrides;

fn view(packs: &[&[(&str, &str)]]) -> LayeredPackView {
    let archives = packs
        .iter()
        .enumerate()
        .map(|(index, files)| {
            let id = format!("00000000-0000-0000-0000-{index:012}");
            let manifest = format!(
                r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
            );
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (path, text) in std::iter::once(("manifest.json", manifest.as_str()))
                .chain(files.iter().copied())
            {
                writer
                    .start_file(path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(text.as_bytes()).unwrap();
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

fn score_file(body: &str) -> String {
    format!(r#"{{"namespace":"scoreboard","scoreboard_sidebar_score":{body}}}"#)
}

#[test]
fn a_pack_without_scoreboard_overrides_changes_nothing() {
    assert!(compile_hud_overrides(&view(&[&[]])).is_none());
    let recolour = score_file(r#"{"color":[1,1,1]}"#);
    assert!(compile_hud_overrides(&view(&[&[("ui/scoreboards.json", &recolour)]])).is_none());
}

#[test]
fn hiding_properties_on_the_score_label_hide_the_column() {
    for body in [
        r#"{"visible":false}"#,
        r#"{"ignored":true}"#,
        r#"{"size":[0,10]}"#,
        r#"{"alpha":0}"#,
        r#"{"text":""}"#,
    ] {
        let file = score_file(body);
        let overrides = compile_hud_overrides(&view(&[&[("ui/scoreboards.json", &file)]]))
            .unwrap_or_else(|| panic!("{body}"));
        assert!(overrides.hide_sidebar_scores, "{body}");
    }
}

// A higher pack's key replaces the lower one's; keys it omits keep the lower value.
#[test]
fn higher_packs_win_per_property_and_base_suffixes_are_ignored() {
    let hidden = r#"{"namespace":"scoreboard","scoreboard_sidebar_score@scoreboard.base":{"visible":false}}"#;
    let shown = score_file(r#"{"visible":true}"#);
    assert!(
        compile_hud_overrides(&view(&[&[("ui/scoreboards.json", hidden)]]))
            .is_some_and(|o| o.hide_sidebar_scores)
    );
    assert!(
        compile_hud_overrides(&view(&[
            &[("ui/scoreboards.json", hidden)],
            &[("ui/scoreboards.json", &shown)]
        ]))
        .is_none()
    );
}

// Local-only: set CINNABAR_SERVER_PACK to a cached server `.mcpack` and print what it overrides.
#[test]
fn a_real_server_pack_is_read() {
    let Some(path) = std::env::var_os("CINNABAR_SERVER_PACK") else {
        return;
    };
    let archive = protocol::ResourcePackArchive::unencrypted(
        "00000000-0000-0000-0000-000000000009".parse().unwrap(),
        "1.0.0".into(),
        String::new(),
        std::fs::read(path).unwrap(),
    );
    let view = LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(vec![archive]),
    ));
    eprintln!("{:?}", compile_hud_overrides(&view));
}
