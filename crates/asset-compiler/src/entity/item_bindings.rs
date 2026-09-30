//! Reviewed default-icon bindings, independent of texture filename spelling.
//!
//! This bounded crosswalk adds canonical inventory keys to the existing atlas
//! routes. It establishes neither auxiliary icon states nor metadata policy.

use std::collections::BTreeSet;

use assets::AssetError;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::invalid;

pub(super) const SOURCE_PATH: &str = "registry/default-sprite-bindings-1.26.40.json";
pub(super) const SOURCE_BYTES: &[u8] =
    include_bytes!("../../../assets/data/default-sprite-bindings-1.26.40.json");
const RETAIL_ITEMS: &[u8] = include_bytes!("../../../protocol/data/retail_items_1_26_50.tsv");
const RETAIL_SHA256: &str = "ee8917e7293c89469d6d114cad634eac0b45a702a1d73e2edddd6d5eeee725d0";
const SOURCE_COMMIT: &str = "7844835b6baad4c0010f46901a4accf87413a022";
const ATLAS_SHA256: &str = "13415a73201c43c03afc7ff9c4e5146366ec31e4f3057228f309730daea963c7";
const ARCHIVE_SHA256: &str = "6f6c3a8d5462cf0fc66fd0782a619a2b1a1f1255b981e7787d05dcfbd363a8cb";
const ROUTE_COUNT: usize = 29;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingTable {
    schema: u32,
    game_version: Box<str>,
    source_tag: Box<str>,
    source_commit: Box<str>,
    source_url: Box<str>,
    archive_sha256: Box<str>,
    atlas_sha256: Box<str>,
    retail_allowlist_sha256: Box<str>,
    coverage: Box<str>,
    routes: Box<[DefaultBinding]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DefaultBinding {
    pub(super) identifier: Box<str>,
    pub(super) default_alias: Box<str>,
    pub(super) atlas_variant: u32,
    evidence_file: Box<str>,
    evidence_sha256: Box<str>,
}

pub(super) fn reviewed() -> Result<Box<[DefaultBinding]>, AssetError> {
    parse(SOURCE_BYTES)
}

fn parse(bytes: &[u8]) -> Result<Box<[DefaultBinding]>, AssetError> {
    let table: BindingTable = serde_json::from_slice(bytes).map_err(|source| AssetError::Json {
        path: SOURCE_PATH.into(),
        source,
    })?;
    let retail_hash = format!("{:x}", Sha256::digest(RETAIL_ITEMS));
    if table.schema != 1
        || table.game_version.as_ref() != "1.26.40"
        || table.source_tag.as_ref() != "v1.26.40.05"
        || table.source_commit.as_ref() != SOURCE_COMMIT
        || table.source_url.as_ref()
            != "https://github.com/Mojang/bedrock-samples/tree/7844835b6baad4c0010f46901a4accf87413a022"
        || table.archive_sha256.as_ref() != ARCHIVE_SHA256
        || table.atlas_sha256.as_ref() != ATLAS_SHA256
        || table.retail_allowlist_sha256.as_ref() != RETAIL_SHA256
        || retail_hash != RETAIL_SHA256
        || table.coverage.is_empty()
        || table.routes.len() != ROUTE_COUNT
    {
        return Err(invalid(
            "default sprite binding provenance does not match reviewed inputs",
        ));
    }
    let retail = std::str::from_utf8(RETAIL_ITEMS)
        .map_err(|_| invalid("retail item allowlist is not UTF-8"))?
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, identifier)| identifier))
        .collect::<BTreeSet<_>>();
    let mut previous: Option<&str> = None;
    for binding in &table.routes {
        if !retail.contains(binding.identifier.as_ref())
            || previous.is_some_and(|value| value >= binding.identifier.as_ref())
            || binding.atlas_variant != 0
            || binding.default_alias.is_empty()
            || binding.default_alias.len() > 256
            || !binding
                .default_alias
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || !binding.evidence_file.starts_with("behavior_pack/items/")
            || !binding.evidence_file.ends_with(".json")
            || binding.evidence_file.contains("..")
            || binding.evidence_file.contains('\\')
            || binding.evidence_sha256.len() != 64
            || !binding
                .evidence_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid(
                "default sprite binding is noncanonical, unsupported or unordered",
            ));
        }
        previous = Some(&binding.identifier);
    }
    Ok(table.routes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn altered(change: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut table = serde_json::from_slice(SOURCE_BYTES).unwrap();
        change(&mut table);
        serde_json::to_vec(&table).unwrap()
    }

    #[test]
    fn reviewed_table_has_exact_coverage_and_rejects_provenance_drift() {
        assert_eq!(reviewed().unwrap().len(), 29);
        for field in [
            "source_commit",
            "atlas_sha256",
            "retail_allowlist_sha256",
            "archive_sha256",
        ] {
            assert!(parse(&altered(|table| table[field] = "wrong".into())).is_err());
        }
    }

    #[test]
    fn duplicate_nonretail_and_nondefault_bindings_fail_closed() {
        assert!(
            parse(&altered(
                |table| table["routes"][1] = table["routes"][0].clone()
            ))
            .is_err()
        );
        assert!(
            parse(&altered(
                |table| table["routes"][0]["identifier"] = "minecraft:invented".into()
            ))
            .is_err()
        );
        assert!(
            parse(&altered(
                |table| table["routes"][0]["atlas_variant"] = 1.into()
            ))
            .is_err()
        );
        assert!(
            parse(&altered(
                |table| table["routes"][0]["default_alias"] = "../apple".into()
            ))
            .is_err()
        );
        assert!(
            parse(&altered(
                |table| table["routes"][0]["evidence_sha256"] = "invalid".into()
            ))
            .is_err()
        );
    }
}
