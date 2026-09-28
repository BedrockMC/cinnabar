use std::path::Path;

use asset_compiler::compile_entity_assets_with_report;
use assets::{
    AssetError, EntityDependencyResolution, EquipmentCategory, EquipmentTransform,
    encode_entity_blob, encode_equipment_catalog,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    MAX_SOURCE_MANIFEST_BYTES, hex, read_bounded_with_limit, validate_output_bundle,
    write_blob_atomic,
};

#[derive(Serialize)]
struct EquipmentAssetsReport {
    schema: u32,
    source: serde_json::Value,
    source_manifest_sha256: Box<str>,
    entity_blob_sha256: Box<str>,
    carrier_sha256: Box<str>,
    counts: EquipmentCounts,
}

#[derive(Serialize)]
struct EquipmentCounts {
    bindings: usize,
    held: usize,
    armor: usize,
    shield: usize,
    elytra: usize,
    catalog_geometry: usize,
    external_geometry: usize,
    literal_first_person: usize,
    literal_third_person: usize,
}

pub(super) fn compile_equipment_assets_command(
    pack: &Path,
    source_manifest: &Path,
    out: &Path,
    report: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_bytes = read_bounded_with_limit(
        source_manifest,
        MAX_SOURCE_MANIFEST_BYTES,
        "source manifest",
    )?;
    let source =
        serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
            AssetError::Json {
                path: source_manifest.to_path_buf(),
                source,
            }
        })?;
    let compilation = compile_entity_assets_with_report(pack, &manifest_bytes)?;
    let entity_blob = encode_entity_blob(&compilation.assets)?;
    let entity_blob_sha256: [u8; 32] = Sha256::digest(&entity_blob).into();
    let bindings = &compilation.equipment_bindings;
    let carrier = encode_equipment_catalog(
        compilation.assets.source_manifest_sha256,
        entity_blob_sha256,
        bindings,
    )?;
    let counts = EquipmentCounts {
        bindings: bindings.len(),
        held: bindings
            .iter()
            .filter(|binding| binding.category == EquipmentCategory::Held)
            .count(),
        armor: bindings
            .iter()
            .filter(|binding| matches!(binding.category, EquipmentCategory::Armor { .. }))
            .count(),
        shield: bindings
            .iter()
            .filter(|binding| binding.category == EquipmentCategory::Shield)
            .count(),
        elytra: bindings
            .iter()
            .filter(|binding| binding.category == EquipmentCategory::Elytra)
            .count(),
        catalog_geometry: bindings
            .iter()
            .filter(|binding| binding.geometry.resolution == EntityDependencyResolution::Catalog)
            .count(),
        external_geometry: bindings
            .iter()
            .filter(|binding| binding.geometry.resolution == EntityDependencyResolution::External)
            .count(),
        literal_first_person: bindings
            .iter()
            .filter(|binding| matches!(binding.first_person, EquipmentTransform::Literal { .. }))
            .count(),
        literal_third_person: bindings
            .iter()
            .filter(|binding| matches!(binding.third_person, EquipmentTransform::Literal { .. }))
            .count(),
    };
    let report_data = EquipmentAssetsReport {
        schema: 1,
        source,
        source_manifest_sha256: hex(&compilation.assets.source_manifest_sha256).into_boxed_str(),
        entity_blob_sha256: hex(&entity_blob_sha256).into_boxed_str(),
        carrier_sha256: format!("{:x}", Sha256::digest(&carrier)).into_boxed_str(),
        counts,
    };
    let mut report_bytes =
        serde_json::to_vec_pretty(&report_data).map_err(|source| AssetError::Json {
            path: report.to_path_buf(),
            source,
        })?;
    report_bytes.push(b'\n');
    validate_output_bundle(out, report)?;
    write_blob_atomic(out, &carrier)?;
    write_blob_atomic(report, &report_bytes)?;
    println!(
        "compiled {} equipment bindings ({} armor, {} held, {} literal FP, {} literal TP) to {} and {}",
        report_data.counts.bindings,
        report_data.counts.armor,
        report_data.counts.held,
        report_data.counts.literal_first_person,
        report_data.counts.literal_third_person,
        out.display(),
        report.display()
    );
    Ok(())
}
