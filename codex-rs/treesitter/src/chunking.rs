use std::path::Path;

use serde::Serialize;

use crate::error::TreeSitterError;
use crate::file_tree::FileTree;

#[derive(Debug, Clone, Serialize)]
pub struct ChunkInfo {
    pub index: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkIndicesResult {
    pub file: String,
    pub total_bytes: usize,
    pub chunk_size: usize,
    pub overlap: usize,
    pub chunks: Vec<ChunkInfo>,
}

pub fn chunk_indices(
    root: &Path,
    file_tree: &FileTree,
    rel_file: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<ChunkIndicesResult, TreeSitterError> {
    if chunk_size == 0 {
        return Err(TreeSitterError::InvalidChunkSize);
    }
    if overlap >= chunk_size {
        return Err(TreeSitterError::InvalidChunkOverlap {
            chunk_size,
            overlap,
        });
    }
    if file_tree.get(rel_file).is_none() {
        return Err(TreeSitterError::PathOutsideRoot {
            path: root.join(rel_file),
        });
    }

    let source = std::fs::read_to_string(root.join(rel_file))?;
    let total_bytes = source.len();
    let step = chunk_size - overlap;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while start < total_bytes {
        let end = (start + chunk_size).min(total_bytes);
        chunks.push(ChunkInfo { index, start, end });
        index += 1;
        if end >= total_bytes {
            break;
        }
        start += step;
    }

    Ok(ChunkIndicesResult {
        file: rel_file.to_string(),
        total_bytes,
        chunk_size,
        overlap,
        chunks,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::file_entry::FileEntry;
    use crate::file_tree::FileTree;

    use super::chunk_indices;

    #[test]
    fn computes_chunk_ranges_with_overlap() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path();
        let rel_file = "src/main.rs";
        fs::create_dir_all(root.join("src")).expect("mkdir");
        fs::write(root.join(rel_file), "abcdefghijklmnop").expect("write");

        let file_tree = FileTree::new();
        file_tree.insert(FileEntry::new(rel_file.to_string(), 16));

        let result = chunk_indices(root, &file_tree, rel_file, 6, 2).expect("chunk indices");

        assert_eq!(result.total_bytes, 16);
        assert_eq!(result.chunks.len(), 4);
        assert_eq!(result.chunks[0].start, 0);
        assert_eq!(result.chunks[0].end, 6);
        assert_eq!(result.chunks[1].start, 4);
        assert_eq!(result.chunks[1].end, 10);
        assert_eq!(result.chunks[2].start, 8);
        assert_eq!(result.chunks[2].end, 14);
        assert_eq!(result.chunks[3].start, 12);
        assert_eq!(result.chunks[3].end, 16);
    }
}
