use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
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
const CACHE_MAX_ENCODED_ENTRY_SIZE: u64 = 10 * 1024 * 1024;
const DEFAULT_FILE_CACHE_CAPACITY: usize = 8;
const FILE_CACHE_CAPACITY_ENV_VAR: &str = "CODEX_FILE_CACHE_CAPACITY";

type FileCacheKey = (PathBuf, u64, u64);

static FILE_CACHE: LazyLock<BlockingLruCache<FileCacheKey, Arc<ProcessedFile>>> =
    LazyLock::new(|| BlockingLruCache::new(file_cache_capacity()));

fn parse_cache_capacity(raw: &str) -> Option<NonZeroUsize> {
    raw.trim().parse::<usize>().ok().and_then(NonZeroUsize::new)
}

fn default_cache_capacity() -> NonZeroUsize {
    NonZeroUsize::new(DEFAULT_FILE_CACHE_CAPACITY).unwrap_or(NonZeroUsize::MIN)
}

fn file_cache_capacity() -> NonZeroUsize {
    std::env::var(FILE_CACHE_CAPACITY_ENV_VAR)
        .ok()
        .as_deref()
        .and_then(parse_cache_capacity)
        .unwrap_or_else(default_cache_capacity)
}

pub fn bytes_to_megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

pub fn file_name_or_default(path: &Path, default_name: &str) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| default_name.to_string())
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
    let filename = file_name_or_default(path, "file");

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
pub fn encode_inline_cached(path: &Path) -> Result<Arc<ProcessedFile>, FileProcessingError> {
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

    let filename = file_name_or_default(path, "file");

    let processed = Arc::new(ProcessedFile {
        base64: BASE64_STANDARD.encode(bytes),
        mime_type,
        filename,
        size_bytes,
    });
    if processed.base64.len() as u64 <= CACHE_MAX_ENCODED_ENTRY_SIZE {
        let _ = FILE_CACHE.insert(key, Arc::clone(&processed));
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
    use std::sync::Arc;
    use std::sync::LazyLock;
    use std::sync::Mutex;

    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn resolves_filename_or_default() {
        let resolved = file_name_or_default(Path::new("/tmp/report.pdf"), "fallback.pdf");
        assert_eq!(resolved, "report.pdf");

        let fallback = file_name_or_default(Path::new("/"), "fallback.pdf");
        assert_eq!(fallback, "fallback.pdf");
    }

    fn write_pdf(path: &Path, body: &[u8]) -> std::io::Result<()> {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(body);
        std::fs::write(path, bytes)
    }

    static CACHE_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock_cache_tests() -> std::sync::MutexGuard<'static, ()> {
        CACHE_TEST_LOCK.lock().expect("cache test lock poisoned")
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
        let _lock = lock_cache_tests();
        FILE_CACHE.clear();

        let file = NamedTempFile::new().expect("temp file");
        write_pdf(file.path(), b"v1").expect("write pdf");

        let first = encode_inline_cached(file.path()).expect("first encode");
        let second = encode_inline_cached(file.path()).expect("second encode");
        assert_eq!(first, second);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_invalidates_on_file_change() {
        let _lock = lock_cache_tests();
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

    #[test]
    fn parse_cache_capacity_rejects_invalid_values() {
        assert_eq!(parse_cache_capacity(""), None);
        assert_eq!(parse_cache_capacity("0"), None);
        assert_eq!(parse_cache_capacity("-1"), None);
        assert_eq!(parse_cache_capacity("abc"), None);
    }

    #[test]
    fn parse_cache_capacity_accepts_positive_values() {
        assert_eq!(parse_cache_capacity("16"), NonZeroUsize::new(16));
        assert_eq!(parse_cache_capacity(" 32 "), NonZeroUsize::new(32));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_handles_concurrent_reads_for_same_file() {
        let _lock = lock_cache_tests();
        FILE_CACHE.clear();

        let file = NamedTempFile::new().expect("temp file");
        write_pdf(file.path(), b"shared").expect("write pdf");
        let expected = encode_inline_cached(file.path()).expect("seed cache");

        let path = file.path().to_path_buf();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            tasks.push(tokio::task::spawn_blocking(move || {
                encode_inline_cached(&path).expect("concurrent encode")
            }));
        }

        for task in tasks {
            let value = task.await.expect("task join");
            assert_eq!(value, expected);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_skips_entries_when_encoded_payload_exceeds_limit() {
        let _lock = lock_cache_tests();
        FILE_CACHE.clear();

        let file = NamedTempFile::new().expect("temp file");
        let body = vec![b'x'; 8 * 1024 * 1024];
        write_pdf(file.path(), &body).expect("write large pdf");

        let first = encode_inline_cached(file.path()).expect("first encode");
        let second = encode_inline_cached(file.path()).expect("second encode");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "encoded payload larger than cache limit should not be cached"
        );
    }
}
