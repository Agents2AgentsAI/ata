use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use codex_lsp_client::lsp_types::AnnotatedTextEdit;
use codex_lsp_client::lsp_types::DocumentChangeOperation;
use codex_lsp_client::lsp_types::DocumentChanges;
use codex_lsp_client::lsp_types::OneOf;
use codex_lsp_client::lsp_types::Position;
use codex_lsp_client::lsp_types::Range;
use codex_lsp_client::lsp_types::ResourceOp;
use codex_lsp_client::lsp_types::TextDocumentEdit;
use codex_lsp_client::lsp_types::TextEdit;
use codex_lsp_client::lsp_types::Uri;
use codex_lsp_client::lsp_types::WorkspaceEdit;
use similar::TextDiff;

#[derive(Debug, Clone, Copy)]
pub struct PatchLimits {
    pub max_files_touched: usize,
    pub max_patch_bytes: usize,
}

impl Default for PatchLimits {
    fn default() -> Self {
        Self {
            max_files_touched: 50,
            max_patch_bytes: 256 * 1024,
        }
    }
}

pub fn workspace_edit_to_apply_patch(
    edit: &WorkspaceEdit,
    limits: PatchLimits,
) -> Result<String, String> {
    let mut ctx = VirtualWorkspace::new();

    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            let path = path_from_uri(uri)?;
            ctx.apply_text_edits(&path, edits)?;
        }
    }

    if let Some(document_changes) = &edit.document_changes {
        match document_changes {
            DocumentChanges::Edits(edits) => {
                for e in edits {
                    apply_text_document_edit(&mut ctx, e)?;
                }
            }
            DocumentChanges::Operations(ops) => {
                for op in ops {
                    match op {
                        DocumentChangeOperation::Edit(e) => apply_text_document_edit(&mut ctx, e)?,
                        DocumentChangeOperation::Op(op) => match op {
                            ResourceOp::Create(create) => {
                                let path = path_from_uri(&create.uri)?;
                                ctx.create_file(&path)?;
                            }
                            ResourceOp::Rename(rename) => {
                                let old_path = path_from_uri(&rename.old_uri)?;
                                let new_path = path_from_uri(&rename.new_uri)?;
                                ctx.rename_file(&old_path, &new_path)?;
                            }
                            ResourceOp::Delete(delete) => {
                                let path = path_from_uri(&delete.uri)?;
                                ctx.delete_file(&path)?;
                            }
                        },
                    }
                }
            }
        }
    }

    build_apply_patch(&mut ctx, limits)
}

fn apply_text_document_edit(
    ctx: &mut VirtualWorkspace,
    e: &TextDocumentEdit,
) -> Result<(), String> {
    let path = path_from_uri(&e.text_document.uri)?;
    let mut edits = Vec::with_capacity(e.edits.len());
    for edit in &e.edits {
        match edit {
            OneOf::Left(text_edit) => edits.push(text_edit.clone()),
            OneOf::Right(AnnotatedTextEdit { text_edit, .. }) => edits.push(text_edit.clone()),
        }
    }
    ctx.apply_text_edits(&path, &edits)
}

fn path_from_uri(uri: &Uri) -> Result<PathBuf, String> {
    let url = url::Url::parse(uri.as_str())
        .map_err(|e| format!("invalid URI '{}': {e}", uri.as_str()))?;
    url.to_file_path()
        .map_err(|_| format!("URI is not a file path: {}", uri.as_str()))
}

#[derive(Default)]
struct VirtualWorkspace {
    original: HashMap<PathBuf, String>,
    current: HashMap<PathBuf, String>,
    created: HashSet<PathBuf>,
    deleted: HashSet<PathBuf>,
    renames: Vec<(PathBuf, PathBuf)>,
}

impl VirtualWorkspace {
    fn new() -> Self {
        Self::default()
    }

    fn ensure_loaded(&mut self, path: &PathBuf) -> Result<(), String> {
        if self.current.contains_key(path) {
            return Ok(());
        }

        let contents = std::fs::read_to_string(path).unwrap_or_default();
        self.original
            .entry(path.clone())
            .or_insert_with(|| contents.clone());
        self.current.insert(path.clone(), contents);
        Ok(())
    }

    fn original_contents(&mut self, path: &PathBuf) -> String {
        if let Some(contents) = self.original.get(path) {
            return contents.clone();
        }
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        self.original.insert(path.clone(), contents.clone());
        contents
    }

    fn create_file(&mut self, path: &PathBuf) -> Result<(), String> {
        self.created.insert(path.clone());
        self.deleted.remove(path);
        if !self.current.contains_key(path) {
            // LSP create ops don't include content; start from empty.
            self.current.insert(path.clone(), String::new());
        }
        Ok(())
    }

    fn delete_file(&mut self, path: &PathBuf) -> Result<(), String> {
        self.deleted.insert(path.clone());
        self.created.remove(path);
        self.current.remove(path);
        Ok(())
    }

    fn rename_file(&mut self, old_path: &PathBuf, new_path: &PathBuf) -> Result<(), String> {
        self.ensure_loaded(old_path)?;

        let contents = self.current.remove(old_path).unwrap_or_default();
        self.current.insert(new_path.clone(), contents);
        self.created.remove(old_path);
        self.deleted.remove(old_path);
        self.deleted.remove(new_path);
        self.renames.push((old_path.clone(), new_path.clone()));
        Ok(())
    }

    fn apply_text_edits(&mut self, path: &PathBuf, edits: &[TextEdit]) -> Result<(), String> {
        self.ensure_loaded(path)?;
        let contents = self.current.get(path).cloned().unwrap_or_default();
        let updated = apply_text_edits_utf16(&contents, edits)?;
        self.current.insert(path.clone(), updated);
        Ok(())
    }
}

fn apply_text_edits_utf16(contents: &str, edits: &[TextEdit]) -> Result<String, String> {
    if edits.is_empty() {
        return Ok(contents.to_string());
    }

    let mut changes: Vec<(usize, usize, String)> = Vec::with_capacity(edits.len());
    for edit in edits {
        let (start, end) = range_to_offsets(contents, edit.range)?;
        changes.push((start, end, edit.new_text.clone()));
    }

    // Apply in reverse so offsets remain valid.
    changes.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out = contents.to_string();
    for (start, end, new_text) in changes {
        if start > end || end > out.len() {
            return Err("invalid edit range".to_string());
        }
        out.replace_range(start..end, &new_text);
    }

    Ok(out)
}

fn range_to_offsets(contents: &str, range: Range) -> Result<(usize, usize), String> {
    let start = position_to_offset_utf16(contents, range.start)?;
    let end = position_to_offset_utf16(contents, range.end)?;
    Ok((start, end))
}

fn position_to_offset_utf16(contents: &str, pos: Position) -> Result<usize, String> {
    let mut line_starts: Vec<usize> = vec![0];
    for (idx, b) in contents.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            line_starts.push(idx + 1);
        }
    }

    let line = pos.line as usize;
    if line >= line_starts.len() {
        return Err(format!(
            "position line {} out of range (max {})",
            line,
            line_starts.len().saturating_sub(1)
        ));
    }

    let line_start = line_starts[line];
    let line_end = line_starts.get(line + 1).copied().unwrap_or(contents.len());
    let line_slice = &contents[line_start..line_end];

    let target_units = pos.character as usize;
    let mut units = 0usize;
    let mut byte_idx = 0usize;
    for (i, ch) in line_slice.char_indices() {
        if units >= target_units {
            byte_idx = i;
            break;
        }
        units += ch.len_utf16();
        byte_idx = i + ch.len_utf8();
    }
    if units < target_units {
        // Clamp to end of line if the server reported a column beyond EOL.
        byte_idx = line_slice.len();
    }

    Ok(line_start + byte_idx)
}

fn build_apply_patch(ctx: &mut VirtualWorkspace, limits: PatchLimits) -> Result<String, String> {
    let mut handled: HashSet<PathBuf> = HashSet::new();
    let mut hunks: Vec<String> = Vec::new();

    let mut rename_sources: HashSet<PathBuf> = HashSet::new();
    let mut rename_dests: HashSet<PathBuf> = HashSet::new();
    for (old_path, new_path) in &ctx.renames {
        rename_sources.insert(old_path.clone());
        rename_dests.insert(new_path.clone());
    }

    // Emit renames first so subsequent updates to the destination path apply cleanly.
    for (old_path, new_path) in ctx.renames.clone() {
        let old_contents = ctx.original_contents(&old_path);
        let new_contents = ctx.current.get(&new_path).cloned().unwrap_or_default();

        if old_contents == new_contents {
            // No textual change; represent as delete+add to avoid producing an empty Update hunk.
            hunks.push(format!("*** Add File: {}", new_path.display()));
            push_add_file_body(&mut hunks, &new_contents);
            hunks.push(format!("*** Delete File: {}", old_path.display()));
        } else {
            let chunks = diff_to_update_chunks(&old_contents, &new_contents)?;
            hunks.push(format!("*** Update File: {}", old_path.display()));
            hunks.push(format!("*** Move to: {}", new_path.display()));
            for chunk in chunks {
                hunks.push("@@".to_string());
                hunks.extend(chunk);
            }
        }

        handled.insert(old_path);
        handled.insert(new_path);
    }

    // Deletes.
    for path in ctx.deleted.clone() {
        if handled.contains(&path) {
            continue;
        }
        hunks.push(format!("*** Delete File: {}", path.display()));
        handled.insert(path);
    }

    // Adds and updates.
    for (path, new_contents) in ctx.current.clone() {
        if handled.contains(&path) {
            continue;
        }
        let exists_on_disk = std::fs::metadata(&path)
            .map(|m| m.is_file())
            .unwrap_or(false);
        if !exists_on_disk || ctx.created.contains(&path) {
            hunks.push(format!("*** Add File: {}", path.display()));
            push_add_file_body(&mut hunks, &new_contents);
            handled.insert(path);
            continue;
        }

        let old_contents = ctx.original_contents(&path);
        if old_contents == new_contents {
            continue;
        }

        let chunks = diff_to_update_chunks(&old_contents, &new_contents)?;
        hunks.push(format!("*** Update File: {}", path.display()));
        for chunk in chunks {
            hunks.push("@@".to_string());
            hunks.extend(chunk);
        }
        handled.insert(path);
    }

    // Enforce limits.
    let file_touches = count_files_touched(&hunks);
    if file_touches > limits.max_files_touched {
        return Err(format!(
            "preview touches {file_touches} files, exceeding limit {}",
            limits.max_files_touched
        ));
    }

    let body = hunks.join("\n");
    let patch = format!("*** Begin Patch\n{body}\n*** End Patch");
    if patch.len() > limits.max_patch_bytes {
        return Err(format!(
            "preview patch is {} bytes, exceeding limit {}",
            patch.len(),
            limits.max_patch_bytes
        ));
    }
    Ok(patch)
}

fn count_files_touched(hunks: &[String]) -> usize {
    hunks
        .iter()
        .filter(|line| {
            line.starts_with("*** Add File: ")
                || line.starts_with("*** Update File: ")
                || line.starts_with("*** Delete File: ")
        })
        .count()
}

fn push_add_file_body(hunks: &mut Vec<String>, contents: &str) {
    for line in contents.split_terminator('\n') {
        hunks.push(format!("+{line}"));
    }
}

fn diff_to_update_chunks(
    old_contents: &str,
    new_contents: &str,
) -> Result<Vec<Vec<String>>, String> {
    let diff = TextDiff::from_lines(old_contents, new_contents);
    let unified = diff.unified_diff().context_radius(2).to_string();

    let mut chunks: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut in_hunk = false;

    for line in unified.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }
        if line.starts_with("@@") {
            if in_hunk {
                if !current.is_empty() {
                    chunks.push(current);
                }
                current = Vec::new();
            }
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        let first = line.chars().next().unwrap_or(' ');
        if first == ' ' || first == '+' || first == '-' {
            current.push(line.to_string());
        } else if line.starts_with("\\ No newline at end of file") {
            // Drop this marker; apply_patch operates on line sequences and normalizes newlines.
        } else {
            // Unknown line; ignore rather than producing an invalid patch.
        }
    }

    if in_hunk && !current.is_empty() {
        chunks.push(current);
    }

    if chunks.is_empty() {
        return Err("failed to generate diff chunks for preview".to_string());
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_apply_patch::apply_patch as apply_patch_to_fs;
    use std::collections::HashMap;

    #[test]
    fn workspace_edit_to_patch_updates_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "world\n").unwrap();

        let uri_str = url::Url::from_file_path(&file_path)
            .unwrap()
            .as_str()
            .to_string();
        let uri: Uri = uri_str.parse().unwrap();

        let edit = WorkspaceEdit {
            changes: Some(HashMap::from([(
                uri,
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    new_text: "hello ".to_string(),
                }],
            )])),
            document_changes: None,
            change_annotations: None,
        };

        let patch = workspace_edit_to_apply_patch(&edit, PatchLimits::default()).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        apply_patch_to_fs(&patch, &mut stdout, &mut stderr).unwrap();

        let contents = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents, "hello world\n");
    }
}
