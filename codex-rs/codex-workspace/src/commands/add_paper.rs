use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::paths;
use crate::resolve::is_reserved_alias;
use crate::types::PaperEntry;
use crate::types::PaperSource;
use crate::types::WorkspaceManifest;
use regex::Regex;
use serde_json::Map;
use sha2::Digest;
use std::path::Path;
use std::sync::LazyLock;

// SAFETY: regex pattern is a compile-time string literal and is known valid.
#[allow(clippy::expect_used)]
static ALIAS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").expect("valid regex"));

pub fn run(
    workspace_id: &str,
    text_md_path: &Path,
    alias: &str,
    title: Option<&str>,
    doi: Option<&str>,
    pdf_path: Option<&Path>,
) -> Result<WorkspaceManifest, WorkspaceError> {
    if is_reserved_alias(alias) {
        return Err(WorkspaceError::ReservedAlias(alias.to_string()));
    }
    if !ALIAS_RE.is_match(alias) {
        return Err(WorkspaceError::InvalidAlias {
            alias: alias.to_string(),
        });
    }
    if !text_md_path.is_file() {
        return Err(WorkspaceError::ManifestNotFound(text_md_path.to_path_buf()));
    }
    if let Some(pdf_path) = pdf_path
        && !pdf_path.is_file()
    {
        return Err(WorkspaceError::ManifestNotFound(pdf_path.to_path_buf()));
    }

    let markdown = std::fs::read_to_string(text_md_path)?;
    let display_title = title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| infer_title(text_md_path, &markdown));
    let content_hash = hash_bytes(markdown.as_bytes());
    let original_markdown_path = text_md_path.to_path_buf();
    let original_pdf_path = pdf_path.map(Path::to_path_buf);
    let alias = alias.to_string();
    let doi = doi.map(ToOwned::to_owned);

    with_locked_manifest(workspace_id, None, move |manifest| {
        if manifest.papers.iter().any(|paper| paper.alias == alias) {
            return Err(WorkspaceError::AliasExists(alias));
        }

        let workspace_root = paths::workspace_root(workspace_id);
        let papers_root = workspace_root.join("papers");
        std::fs::create_dir_all(&papers_root)?;
        let markdown_dest = papers_root.join(format!("{alias}.md"));
        std::fs::copy(&original_markdown_path, &markdown_dest)?;

        let pdf_dest = if let Some(original_pdf_path) = &original_pdf_path {
            let dest = papers_root.join(format!("{alias}.pdf"));
            std::fs::copy(original_pdf_path, &dest)?;
            Some(path_relative_to_workspace(workspace_id, &dest)?)
        } else {
            None
        };

        manifest.papers.push(PaperEntry {
            id: format!("paper.{alias}"),
            alias: alias.clone(),
            title: display_title.clone(),
            doi: doi.clone(),
            source: PaperSource::LocalFile {
                original_path: original_markdown_path.to_string_lossy().to_string(),
            },
            text_md_path: path_relative_to_workspace(workspace_id, &markdown_dest)?,
            pdf_path: pdf_dest,
            content_hash: content_hash.clone(),
            added_at: chrono::Utc::now().timestamp(),
            tags: Vec::new(),
            extra: Map::new(),
        });
        Ok(())
    })
}

fn infer_title(path: &Path, markdown: &str) -> String {
    markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Untitled paper".to_string())
}

fn path_relative_to_workspace(workspace_id: &str, path: &Path) -> Result<String, WorkspaceError> {
    Ok(path
        .strip_prefix(paths::workspace_root(workspace_id))
        .map_err(|_| WorkspaceError::PathEscape(path.display().to_string()))?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_title_prefers_first_heading_then_file_stem() {
        assert_eq!(
            infer_title(Path::new("/tmp/test.md"), "# Test Paper\n\nBody\n"),
            "Test Paper"
        );
        assert_eq!(infer_title(Path::new("/tmp/fallback.md"), ""), "fallback");
    }
}
