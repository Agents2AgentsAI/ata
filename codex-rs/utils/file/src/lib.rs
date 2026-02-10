use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use codex_utils_cache::BlockingLruCache;

pub mod error;

pub use error::FileProcessingError;

/// Maximum processable file size (50 MB, matching current cross-provider PDF processing limits).
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

/// Bandwidth-economic threshold for always-inline file routing.
pub const ALWAYS_INLINE_MAX: u64 = 2 * 1024 * 1024;

const PDF_MAGIC: &[u8] = b"%PDF-";
const CACHE_MAX_ENTRY_SIZE: u64 = 10 * 1024 * 1024;
const FILE_CACHE_CAPACITY: usize = 8;

type FileCacheKey = (PathBuf, u64, u64);

static FILE_CACHE: LazyLock<BlockingLruCache<FileCacheKey, ProcessedFile>> = LazyLock::new(|| {
    BlockingLruCache::new(NonZeroUsize::new(FILE_CACHE_CAPACITY).unwrap_or(NonZeroUsize::MIN))
});

pub fn bytes_to_megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// Processed file data ready for model input.
///
/// This stores only base64 data (not raw bytes) to avoid duplicating memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessedFile {
    pub base64: String,
    pub mime_type: String,
    pub filename: String,
    pub size_bytes: u64,
}

/// Lightweight file metadata from stat + magic byte inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub mime_type: String,
    pub filename: String,
    pub size_bytes: u64,
}

/// Analyze a file with stat + magic bytes only.
///
/// This does not read the full file into memory and does not encode base64.
pub fn analyze_file(path: &Path) -> Result<FileMetadata, FileProcessingError> {
    let metadata = std::fs::metadata(path).map_err(|source| FileProcessingError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let size_bytes = metadata.len();
    if size_bytes > MAX_FILE_SIZE {
        return Err(FileProcessingError::TooLarge {
            path: path.to_path_buf(),
            size_mb: size_bytes as f64 / (1024.0 * 1024.0),
            max_mb: MAX_FILE_SIZE / (1024 * 1024),
        });
    }

    let mut buf = [0u8; 16];
    let bytes_read = {
        let mut file = std::fs::File::open(path).map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        file.read(&mut buf)
            .map_err(|source| FileProcessingError::Read {
                path: path.to_path_buf(),
                source,
            })?
    };

    let mime_type = detect_mime(&buf[..bytes_read], path)?;
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    Ok(FileMetadata {
        mime_type,
        filename,
        size_bytes,
    })
}

/// Read and base64-encode a file.
pub fn encode_inline(
    path: &Path,
    metadata: &FileMetadata,
) -> Result<ProcessedFile, FileProcessingError> {
    let bytes = std::fs::read(path).map_err(|source| FileProcessingError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(ProcessedFile {
        base64: BASE64_STANDARD.encode(bytes),
        mime_type: metadata.mime_type.clone(),
        filename: metadata.filename.clone(),
        size_bytes: metadata.size_bytes,
    })
}

/// Analyze and encode a file with cache lookup by canonical path + mtime + file size.
pub fn encode_inline_cached(path: &Path) -> Result<ProcessedFile, FileProcessingError> {
    // Canonical path keeps cache keys stable across equivalent path spellings/symlinks.
    let canonical = std::fs::canonicalize(path).map_err(|source| FileProcessingError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    // Open once and use the same handle for metadata, header probe, and full read.
    // This avoids TOCTOU windows between independent filesystem operations.
    let mut file = std::fs::File::open(&canonical).map_err(|source| FileProcessingError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let size_bytes = metadata.len();
    if size_bytes > MAX_FILE_SIZE {
        return Err(FileProcessingError::TooLarge {
            path: path.to_path_buf(),
            size_mb: size_bytes as f64 / (1024.0 * 1024.0),
            max_mb: MAX_FILE_SIZE / (1024 * 1024),
        });
    }

    let mtime_ns = file_mtime_ns_from_metadata(&metadata);
    let key = (canonical, mtime_ns, size_bytes);

    if let Some(cached) = FILE_CACHE.get(&key) {
        return Ok(cached);
    }

    let mut mime_probe = [0_u8; 16];
    let bytes_read = file
        .read(&mut mime_probe)
        .map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let mime_type = detect_mime(&mime_probe[..bytes_read], path)?;

    file.seek(SeekFrom::Start(0))
        .map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::with_capacity(size_bytes as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?;

    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let processed = ProcessedFile {
        base64: BASE64_STANDARD.encode(bytes),
        mime_type,
        filename,
        size_bytes,
    };
    if processed.size_bytes <= CACHE_MAX_ENTRY_SIZE {
        let _ = FILE_CACHE.insert(key, processed.clone());
    }
    Ok(processed)
}

#[cfg(test)]
fn file_mtime_ns(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    file_mtime_ns_from_metadata(&metadata)
}

fn file_mtime_ns_from_metadata(metadata: &std::fs::Metadata) -> u64 {
    let Ok(modified) = metadata.modified() else {
        return 0;
    };
    let Ok(duration) = modified.duration_since(UNIX_EPOCH) else {
        return 0;
    };
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}

fn detect_mime(bytes: &[u8], path: &Path) -> Result<String, FileProcessingError> {
    if bytes.len() >= PDF_MAGIC.len() && &bytes[..PDF_MAGIC.len()] == PDF_MAGIC {
        return Ok("application/pdf".to_string());
    }
    Err(FileProcessingError::UnsupportedType {
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;

    use super::*;

    fn write_pdf(path: &Path, body: &[u8]) -> std::io::Result<()> {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(body);
        std::fs::write(path, bytes)
    }

    #[test]
    fn validates_pdf_magic_bytes() {
        let file = NamedTempFile::new().expect("temp file");
        write_pdf(file.path(), b"test").expect("write pdf");

        let metadata = analyze_file(file.path()).expect("analyze file");
        assert_eq!(metadata.mime_type, "application/pdf");
        assert_eq!(
            metadata.filename,
            file.path()
                .file_name()
                .expect("filename")
                .to_string_lossy()
                .to_string()
        );
    }

    #[test]
    fn rejects_non_pdf() {
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(b"not a pdf").expect("write non pdf");

        let err = analyze_file(file.path()).expect_err("expected unsupported type");
        assert!(matches!(err, FileProcessingError::UnsupportedType { .. }));
    }

    #[test]
    fn rejects_oversized_file_without_full_read() {
        let file = NamedTempFile::new().expect("temp file");
        file.as_file()
            .set_len(MAX_FILE_SIZE + 1)
            .expect("set oversized length");

        let err = analyze_file(file.path()).expect_err("expected too large");
        assert!(matches!(err, FileProcessingError::TooLarge { .. }));
    }

    #[test]
    fn base64_encodes_correctly() {
        let file = NamedTempFile::new().expect("temp file");
        let pdf_bytes = b"%PDF-1.4\n<dummy pdf content>";
        std::fs::write(file.path(), pdf_bytes).expect("write pdf");

        let metadata = analyze_file(file.path()).expect("analyze file");
        let processed = encode_inline(file.path(), &metadata).expect("encode inline");
        assert_eq!(
            processed.base64,
            BASE64_STANDARD.encode(pdf_bytes),
            "base64 payload should match source bytes"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_returns_same_result_for_unchanged_file() {
        FILE_CACHE.clear();

        let file = NamedTempFile::new().expect("temp file");
        write_pdf(file.path(), b"v1").expect("write pdf");

        let first = encode_inline_cached(file.path()).expect("first encode");
        let second = encode_inline_cached(file.path()).expect("second encode");
        assert_eq!(first, second);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_invalidates_on_file_change() {
        FILE_CACHE.clear();

        let file = NamedTempFile::new().expect("temp file");
        write_pdf(file.path(), b"version one").expect("write version one");
        let first = encode_inline_cached(file.path()).expect("first encode");
        let first_mtime_ns = file_mtime_ns(file.path());

        write_pdf(file.path(), b"version two").expect("write version two");
        for _ in 0..30 {
            if file_mtime_ns(file.path()) != first_mtime_ns {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            write_pdf(file.path(), b"version two").expect("rewrite version two");
        }
        let second = encode_inline_cached(file.path()).expect("second encode");

        assert_ne!(first.base64, second.base64);
        assert_eq!(first.mime_type, second.mime_type);
    }
}
