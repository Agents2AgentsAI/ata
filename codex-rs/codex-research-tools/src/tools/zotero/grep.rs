use std::collections::HashMap;

use crate::ResearchToolkit;
use crate::clients::zotero;
use crate::types::ZoteroGrepCandidateStrategy;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepMatchMode;

use super::DEFAULT_CHILDREN_LIMIT;
use super::NormalizedScope;
use super::to_scope;
use super::zotero_config;

const DEFAULT_GREP_ANNOTATION_PREFETCH_MAX_PAGES: usize = 8;

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct NormalizedGrepParams {
    pub(super) pattern: String,
    pub(super) match_mode: ZoteroGrepMatchMode,
    pub(super) case_sensitive: bool,
    pub(super) scope_explicit: bool,
    pub(super) scope: NormalizedScope,
    pub(super) parent_item_key: Option<String>,
    pub(super) query_hint: Option<String>,
    pub(super) item_type: Option<String>,
    pub(super) fields: Vec<ZoteroGrepField>,
    pub(super) limit_items: u32,
    pub(super) limit_matches: u32,
    pub(super) max_matches_per_item: u32,
    pub(super) context_chars: u32,
    pub(super) max_chars_per_item: Option<u32>,
    pub(super) candidate_strategy: ZoteroGrepCandidateStrategy,
}

#[derive(Debug, Clone)]
pub(super) struct GrepCandidate {
    pub(super) key: String,
    pub(super) title: String,
    pub(super) item_type: String,
    pub(super) tags: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct LibraryAnnotationPrefetch {
    pub(super) by_parent: HashMap<String, Vec<zotero::ZoteroAnnotation>>,
    pub(super) complete: bool,
}

pub(super) fn field_enabled(fields: &[ZoteroGrepField], field: ZoteroGrepField) -> bool {
    fields.contains(&field)
}

pub(super) async fn prefetch_library_annotations(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> (LibraryAnnotationPrefetch, Vec<String>) {
    let mut warnings = Vec::new();
    let mut prefetch = LibraryAnnotationPrefetch::default();

    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut offset = 0u32;
    let mut page_count = 0usize;

    loop {
        if page_count >= DEFAULT_GREP_ANNOTATION_PREFETCH_MAX_PAGES {
            warnings.push(
                "annotation prefetch hit page cap; falling back to per-item annotation fetch for uncovered items"
                    .to_string(),
            );
            break;
        }

        let page = match zotero::get_library_annotations(
            toolkit.http(),
            config,
            &scope,
            zotero::ZoteroLibraryAnnotationsRequest {
                offset,
                limit: DEFAULT_CHILDREN_LIMIT,
            },
        )
        .await
        {
            Ok(page) => page,
            Err(err) => {
                warnings.push(format!("annotation prefetch failed: {err}"));
                break;
            }
        };

        page_count = page_count.saturating_add(1);
        let fetched_count = page.annotations.len();
        for annotation in page.annotations {
            if let Some(parent_item) = annotation.parent_item.clone() {
                prefetch
                    .by_parent
                    .entry(parent_item)
                    .or_default()
                    .push(annotation);
            }
        }

        if !page.has_more {
            prefetch.complete = true;
            break;
        }
        if fetched_count == 0 {
            warnings.push(
                "annotation prefetch stopped early because upstream returned an empty page with has_more=true"
                    .to_string(),
            );
            break;
        }

        offset = offset.saturating_add(u32::try_from(fetched_count).unwrap_or(0));
    }

    (prefetch, warnings)
}
