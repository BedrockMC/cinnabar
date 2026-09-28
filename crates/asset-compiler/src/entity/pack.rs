//! In-memory entity compile for a server pack's client entity families. Unlike the
//! vanilla carrier build it is lenient per file: an unparsable or oversized
//! source is skipped and counted, and items and equipment are not compiled.

use super::*;

/// Sources one pack may contribute before the rest are ignored.
pub const MAX_PACK_ENTITY_SOURCES: usize = 4_096;
/// Total source bytes one pack may contribute before the rest are ignored.
pub const MAX_PACK_ENTITY_BYTES: usize = 128 * 1024 * 1024;

/// Directories whose files feed the entity catalog, with their extensions.
const FAMILIES: [(&str, &[&str]); 6] = [
    ("entity/", &["json"]),
    ("models/entity/", &["json"]),
    ("animations/", &["json"]),
    ("animation_controllers/", &["json"]),
    ("render_controllers/", &["json"]),
    ("textures/entity/", &["json", "png", "tga"]),
];

/// Counted reasons pack sources were left out of the catalog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EntityPackSkips {
    pub oversized: u32,
    pub unparsable: u32,
    /// Sources past the count or byte bound.
    pub over_budget: u32,
}

/// A pack's entity catalog. Symbols a pack shares with vanilla resolve as
/// external within it, so rigs that depend on vanilla clips are attributed in
/// `reference_outcomes` rather than compiled.
#[derive(Debug)]
pub struct EntityPackCompilation {
    pub assets: CompiledEntityAssets,
    pub reference_outcomes: Box<[CompileReferenceOutcome<u32>]>,
    pub skipped: EntityPackSkips,
}

fn in_families(path: &str) -> bool {
    FAMILIES.iter().any(|(prefix, extensions)| {
        path.starts_with(prefix)
            && path
                .rsplit_once('.')
                .is_some_and(|(_, extension)| extensions.contains(&extension))
    })
}

/// Compiles `(pack-relative path, bytes)` files; `Ok(None)` when no usable
/// entity source remains, `Err` when the surviving set is structurally invalid.
pub fn compile_entity_pack(
    files: Vec<(Box<str>, Vec<u8>)>,
) -> Result<Option<EntityPackCompilation>, AssetError> {
    let mut selected = files
        .into_iter()
        .filter(|(path, _)| in_families(path))
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.0.cmp(&right.0));
    selected.dedup_by(|later, earlier| later.0 == earlier.0);

    let mut skipped = EntityPackSkips::default();
    let mut sources = Vec::new();
    let mut payloads = SourcePayloads::new();
    let mut symbols = BTreeMap::new();
    let mut geometries = BTreeMap::new();
    let mut total = 0usize;
    for (path, bytes) in selected {
        if bytes.len() > assets::MAX_ENTITY_SOURCE_BYTES {
            skipped.oversized += 1;
            continue;
        }
        let next_total = total.saturating_add(bytes.len());
        if sources.len() >= MAX_PACK_ENTITY_SOURCES || next_total > MAX_PACK_ENTITY_BYTES {
            skipped.over_budget += 1;
            continue;
        }
        // Parse into scratch maps so a failing file leaves no partial symbols.
        let mut file_symbols = BTreeMap::new();
        let mut file_geometries = BTreeMap::new();
        if parse_source(
            &path,
            Path::new(path.as_ref()),
            &bytes,
            &mut file_symbols,
            &mut file_geometries,
        )
        .is_err()
        {
            skipped.unparsable += 1;
            continue;
        }
        total = next_total;
        symbols.extend(file_symbols);
        geometries.extend(file_geometries);
        sources.push(EntityAssetSource {
            path: path.clone(),
            source_bytes: bytes.len() as u32,
            source_sha256: Sha256::digest(&bytes).into(),
        });
        payloads.insert(path, bytes.into_boxed_slice());
    }
    if symbols.is_empty() {
        return Ok(None);
    }
    let mut identity = Sha256::new();
    for source in &sources {
        identity.update(source.path.as_bytes());
        identity.update(source.source_sha256);
    }
    let compilation = assemble(
        Path::new(""),
        sources,
        payloads,
        symbols,
        geometries,
        identity.finalize().into(),
        false,
    )?;
    Ok(Some(EntityPackCompilation {
        assets: compilation.assets,
        reference_outcomes: compilation.reference_outcomes,
        skipped,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> (Box<str>, Vec<u8>) {
        (path.into(), text.as_bytes().to_vec())
    }

    // Bad and out-of-family files are dropped and counted; nothing usable yields None.
    #[test]
    fn unusable_sources_are_skipped_and_counted() {
        let result = compile_entity_pack(vec![
            file("entity/bad.json", "{not json"),
            file("ui/other.json", "{}"),
        ])
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn family_filter_matches_directory_and_extension() {
        assert!(in_families("entity/a.json"));
        assert!(in_families("textures/entity/a/b.png"));
        assert!(!in_families("entity/a.png"));
        assert!(!in_families("textures/blocks/a.png"));
    }
}
