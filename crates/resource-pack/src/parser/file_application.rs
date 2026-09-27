//! Optional one-shot application preparation within the original archive lifetime.

use crate::ValidatedPack;
#[cfg(feature = "handoff")]
use crate::{AdmissionError, ValidatedPackStack};
#[cfg(feature = "handoff")]
use protocol::ResourcePackArchive;
use std::{
    io::{Cursor, Read},
    sync::Arc,
};
use zip::{CompressionMethod, ZipArchive};

pub(super) struct FilePreparation<'a, F, T> {
    path: &'a str,
    limit: usize,
    prepare: Option<F>,
    result: Option<T>,
}

type PreparationFn = fn(usize, &mut dyn FnMut(&mut [u8]) -> bool) -> Option<()>;

pub(super) fn disabled_file() -> FilePreparation<'static, PreparationFn, ()> {
    FilePreparation {
        path: "",
        limit: 0,
        prepare: None,
        result: None,
    }
}

fn language_compression_supported(method: CompressionMethod) -> bool {
    matches!(
        method,
        CompressionMethod::Stored | CompressionMethod::Deflated
    )
}

#[cfg(feature = "handoff")]
pub(crate) fn validate_archives_with_file<T>(
    archives: Vec<ResourcePackArchive>,
    path: &str,
    limit: usize,
    prepare: impl FnOnce(usize, &mut dyn FnMut(&mut [u8]) -> bool) -> Option<T>,
) -> Result<(ValidatedPackStack, Option<T>), AdmissionError> {
    let mut file = FilePreparation {
        path,
        limit,
        prepare: (archives.len() == 1).then_some(prepare),
        result: None,
    };
    let stack = super::validate_archives_inner(archives, &mut file)?;
    Ok((stack, file.result))
}

impl<F, T> FilePreparation<'_, F, T>
where
    F: FnOnce(usize, &mut dyn FnMut(&mut [u8]) -> bool) -> Option<T>,
{
    pub(super) fn apply(&mut self, pack: &ValidatedPack, zip: &mut ZipArchive<Cursor<Arc<[u8]>>>) {
        if let Some(prepare) = self.prepare.take()
            && let Some(entry) = pack.files.get(self.path)
            && entry.uncompressed_size <= self.limit as u64
            && let Ok(size) = usize::try_from(entry.uncompressed_size)
        {
            let mut attempted = false;
            let mut read = |output: &mut [u8]| {
                if std::mem::replace(&mut attempted, true) || output.len() != size {
                    return false;
                }
                // Check the method before constructing a decompressor, independent of features.
                let Ok(raw) = zip.by_index_raw(entry.archive_index) else {
                    return false;
                };
                if !language_compression_supported(raw.compression()) {
                    return false;
                }
                drop(raw);
                let Ok(mut input) = zip.by_index(entry.archive_index) else {
                    return false;
                };
                if input.size() != size as u64 || input.read_exact(output).is_err() {
                    return false;
                }
                let mut eof = [0; 1];
                matches!(input.read(&mut eof), Ok(0))
            };
            self.result = prepare(size, &mut read);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_reader_refuses_other_codecs_independent_of_enabled_features() {
        assert!(language_compression_supported(CompressionMethod::Stored));
        assert!(language_compression_supported(CompressionMethod::Deflated));
        assert!(!language_compression_supported(CompressionMethod::BZIP2));
        assert!(!language_compression_supported(CompressionMethod::LZMA));
        assert!(!language_compression_supported(CompressionMethod::ZSTD));
    }
}
