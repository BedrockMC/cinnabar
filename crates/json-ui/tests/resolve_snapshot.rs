//! Resolution snapshots of the real vanilla screens. The `.local` pack is
//! gitignored, so each test skips (not fails) when it is absent.

mod support;

use std::fmt::Write as _;

use json_ui::{Catalog, Context, DataSource, ENGINE_SCREENS, ResolvedControl, bind_screen};

fn catalog() -> Option<Catalog> {
    let dir = support::vanilla_pack().join("ui");
    dir.is_dir()
        .then(|| Catalog::load_dir(&dir).expect("index files load"))
}

fn dump(control: &ResolvedControl, depth: usize, out: &mut String) {
    let properties = serde_json::to_string(&control.properties).unwrap_or_default();
    let factory = serde_json::to_string(&control.factory).unwrap_or_default();
    let _ = writeln!(
        out,
        "{:indent$}{} type={:?} base={:?} unresolved={:?} factory={factory} props={properties}",
        "",
        control.name,
        control.control_type,
        control.base.as_ref().map(ToString::to_string),
        control.unresolved_base,
        indent = depth * 2
    );
    for child in &control.children {
        dump(child, depth + 1, out);
    }
}

/// Writes every engine screen's resolved and bound tree to `$JSON_UI_DUMP`.
#[test]
#[ignore = "diagnostic dump"]
fn dump_engine_screens() {
    let (Some(catalog), Ok(path)) = (catalog(), std::env::var("JSON_UI_DUMP")) else {
        return;
    };
    let context = Context::retail(true);
    let mut out = String::new();
    let _ = writeln!(out, "== catalog diagnostics");
    for line in catalog.diagnostics() {
        let _ = writeln!(out, "{line}");
    }
    for reference in ENGINE_SCREENS {
        let resolution = json_ui::resolve(&catalog, reference, &context);
        let _ = writeln!(out, "== {reference}");
        let Some(root) = resolution.control else {
            let _ = writeln!(out, "(unresolved)");
            continue;
        };
        dump(&root, 0, &mut out);
        let _ = writeln!(out, "== bound {reference}");
        let bound = bind_screen(&root, &catalog, &context, &DataSource::new());
        dump(&bound, 0, &mut out);
        let mut diagnostics = resolution.diagnostics;
        diagnostics.sort();
        let _ = writeln!(out, "== diagnostics {reference}");
        for line in diagnostics {
            let _ = writeln!(out, "{line}");
        }
    }
    std::fs::write(path, out).unwrap();
}
