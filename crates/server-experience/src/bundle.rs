//! Complete manifest/content verification before any component is compiled.

use std::{collections::{BTreeMap, BTreeSet}, io::{Cursor, Read}};
use anyhow::{Result, ensure};
use zip::{CompressionMethod, ZipArchive};
use crate::{crypto::{self, SignedDocument}, manifest::{Manifest, PackageOffer, Scope}, policy::*};

pub const MANIFEST_PATH: &str = "manifest.signed.json";

#[derive(Debug)]
pub struct VerifiedBundle {
    pub manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

impl VerifiedBundle {
    /// Accepts only indexed regular files, without extracting paths to disk.
    pub fn read(
        bytes: &[u8],
        offer: &PackageOffer,
        scope: &Scope,
        remaining_expanded: u64,
    ) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BUNDLE_BYTES && bytes.len() as u64 == offer.bytes, "bundle size mismatch");
        ensure!(crypto::digest(bytes) == offer.digest, "bundle hash mismatch");
        let mut zip = ZipArchive::new(Cursor::new(bytes))?;
        ensure!(zip.len() <= MAX_FILES + 1, "too many archive entries");
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let file = zip.by_index(index)?;
            ensure!(safe_path(file.name()), "unsafe archive path");
            ensure!(names.insert(file.name().to_ascii_lowercase()), "duplicate archive path");
            ensure!(file.unix_mode().is_none_or(|mode| mode & 0o170000 == 0 || mode & 0o170000 == 0o100000), "nonregular archive entry");
            ensure!(!file.encrypted(), "encrypted bundles are unsupported");
            ensure!(matches!(file.compression(), CompressionMethod::Stored | CompressionMethod::Deflated), "unsupported compression");
            total = total.checked_add(file.size()).ok_or_else(|| anyhow::anyhow!("archive size overflow"))?;
            ensure!(total <= remaining_expanded.min(MAX_EXPANDED_BYTES), "expanded archive limit exceeded");
        }
        let signed = read_entry(&mut zip, MANIFEST_PATH, MAX_MARKER_BYTES as u64)?;
        let signed: SignedDocument = serde_json::from_slice(&signed)?;
        let (manifest, _): (Manifest, _) = signed.verify(&offer.publisher_key, crypto::MANIFEST_DOMAIN, MAX_MARKER_BYTES / 2)?;
        ensure!(manifest.version == WIRE_VERSION && manifest.api == API_VERSION, "unsupported manifest API");
        ensure!(manifest.id == offer.id && manifest.publisher_key == offer.publisher_key, "publisher substitution");
        ensure!(manifest.permissions.is_subset(&scope.permissions), "undeclared permission");
        ensure!(manifest.channels.len() <= MAX_CHANNELS && manifest.actions.len() <= 32, "declaration limit exceeded");
        let mut channels = BTreeSet::new();
        for channel in &manifest.channels {
            ensure!(channel.id.starts_with(&format!("{}.", manifest.id))
                && crate::manifest::identifier(&channel.id)
                && channels.insert((&channel.id, channel.schema)) && channel.fields.len() <= 64,
                "invalid channel declaration");
        }
        ensure!(manifest.actions.iter().all(|id| crate::manifest::identifier(id)), "invalid action declaration");
        ensure!(manifest.files.len() + 1 == zip.len(), "unindexed archive entry");
        let mut files = BTreeMap::new();
        for entry in &manifest.files {
            ensure!(safe_path(&entry.path) && entry.path != MANIFEST_PATH, "invalid index path");
            crypto::fixed_hex::<32>(&entry.sha256)?;
            ensure!(entry.bytes <= MAX_EXPANDED_BYTES, "file size exceeded");
            let data = read_entry(&mut zip, &entry.path, entry.bytes)?;
            ensure!(data.len() as u64 == entry.bytes && crypto::digest(&data) == entry.sha256, "content hash mismatch");
            ensure!(files.insert(entry.path.clone(), data).is_none(), "duplicate index entry");
        }
        if let Some(component) = &manifest.component {
            let data = files.get(component).ok_or_else(|| anyhow::anyhow!("component missing from index"))?;
            ensure!(data.len() <= MAX_COMPONENT_BYTES, "component too large");
            ensure!(data.starts_with(b"\0asm"), "only portable WebAssembly is accepted");
        }
        Ok(Self { manifest, files })
    }

    /// Reads only an immutable, already verified bundle asset.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// Returns the optional portable component, never serialized native code.
    pub fn component(&self) -> Option<&[u8]> {
        self.manifest.component.as_deref().and_then(|path| self.file(path))
    }

    /// Counts actual retained file bytes for the aggregate session budget.
    pub fn expanded_bytes(&self) -> u64 {
        self.files.values().map(|bytes| bytes.len() as u64).sum()
    }

    /// Lists only signed and hash-verified asset identities.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

/// Permits a narrow portable path subset with no ambiguous aliases.
fn safe_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= 256 && path.split('/').count() <= 8
        && path.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'.' | b'_' | b'-'))
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Checks declared size before decompression and caps reads independently.
fn read_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, path: &str, limit: u64) -> Result<Vec<u8>> {
    let file = zip.by_name(path)?;
    ensure!(file.size() <= limit, "entry exceeds declared size");
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "entry expanded beyond limit");
    Ok(bytes)
}
