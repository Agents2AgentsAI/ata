use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_features::Feature;
use codex_protocol::document_reader::AddDocumentSectionArgs;
use codex_protocol::document_reader::AddDocumentSectionEvent;
use codex_protocol::document_reader::AppendDocumentSectionEvent;
use codex_protocol::document_reader::AppendToSectionArgs;
use codex_protocol::document_reader::PatchDocumentSectionArgs;
use codex_protocol::document_reader::PatchDocumentSectionEvent;
use codex_protocol::document_reader::PresentDocumentArgs;
use codex_protocol::document_reader::PresentDocumentEvent;
use codex_protocol::document_reader::UpdateDocumentSectionArgs;
use codex_protocol::document_reader::UpdateDocumentSectionEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SessionSource;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use regex_lite::Regex;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

/// Strip web-browsing citation markers like `citeturn5view0`,
/// `citeturn1view0turn2view3`, or `citeturn5search0` that the model
/// sometimes injects into reading-view content. These are internal
/// artifacts that appear as garbage to the user.
fn strip_citation_markers(text: &str) -> String {
    static RE: LazyLock<Regex> =
        LazyLock::new(
            || match Regex::new(r"\s*cite(?:turn\d+(?:view|search)\d+)+") {
                Ok(re) => re,
                Err(err) => panic!("invalid citation regex: {err}"),
            },
        );
    RE.replace_all(text, "").into_owned()
}

/// Strip any agent-authored `<!-- CODEX_SECTION_META … -->` blocks from
/// content the agent provides via update/append/add/patch tool calls.
///
/// `foldable` and `summary` are expected as separate tool parameters,
/// and the metadata comment is added by `serialize_section_metadata`
/// when the section is rendered. When the agent embeds the comment
/// inline, it has historically supplied a malformed multi-line JSON
/// (with literal newlines inside the `summary` value). The on-disk
/// parser at `parse_section_metadata_line` only recognizes the comment
/// when the prefix and suffix are on the SAME line, so the malformed
/// block is treated as visible content and leaks the entire summary
/// payload into the rendered section.
///
/// Strip both well-formed (single-line) and malformed (multi-line)
/// metadata comments from agent-supplied content. Any `foldable` /
/// `summary` set via the tool parameters wins.
fn strip_agent_authored_metadata(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        // `(?s)` makes `.` match newlines so we catch the multi-line
        // malformed form. Non-greedy `.*?` so we don't eat past the
        // first closing ` -->`.
        match Regex::new(r"(?s)<!--\s*CODEX_SECTION_META\s.*?-->\s*\n?") {
            Ok(re) => re,
            Err(err) => panic!("invalid CODEX_SECTION_META strip regex: {err}"),
        }
    });
    RE.replace_all(text, "").into_owned()
}

// ---------------------------------------------------------------------------
// Cached document state
// ---------------------------------------------------------------------------

struct CachedSection {
    heading: String,
    content: String,
    foldable: bool,
    summary: Option<String>,
}

struct CachedDocument {
    title: String,
    sections: Vec<CachedSection>,
    /// When streaming, tracks the next section index to fill.
    /// `Some(n)` means sections `n..` still need content.
    streaming_next: Option<usize>,
}

impl CachedDocument {
    /// Reconstruct full markdown from cached sections.
    fn to_markdown(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if !section.heading.is_empty() {
                out.push_str("## ");
                out.push_str(&section.heading);
                out.push('\n');
            }
            if let Some(metadata) = serialize_section_metadata(section) {
                out.push_str(&metadata);
                out.push('\n');
            }
            out.push_str(&section.content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }
}

/// Parse markdown content into sections split on `## ` headings.
fn parse_sections(content: &str) -> Vec<CachedSection> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_content = String::new();

    for line in content.lines() {
        if let Some(heading_text) = line.strip_prefix("## ") {
            let (metadata, visible_content) = split_section_metadata(current_content);
            sections.push(CachedSection {
                heading: current_heading,
                content: visible_content,
                foldable: metadata.foldable,
                summary: metadata.summary,
            });
            current_heading = heading_text.trim().to_string();
            current_content = String::new();
        } else {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            current_content.push_str(line);
        }
    }

    let (metadata, visible_content) = split_section_metadata(current_content);
    sections.push(CachedSection {
        heading: current_heading,
        content: visible_content,
        foldable: metadata.foldable,
        summary: metadata.summary,
    });

    // Drop the empty preamble section when the document starts with `## `.
    if sections.len() > 1 && sections[0].heading.is_empty() && sections[0].content.trim().is_empty()
    {
        sections.remove(0);
    }

    sections
}

const SECTION_METADATA_PREFIX: &str = "<!-- CODEX_SECTION_META ";
const SECTION_METADATA_SUFFIX: &str = " -->";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SectionMetadata {
    #[serde(default)]
    foldable: bool,
    #[serde(default)]
    summary: Option<String>,
}

fn serialize_section_metadata(section: &CachedSection) -> Option<String> {
    let metadata = SectionMetadata {
        foldable: section.foldable,
        summary: section.summary.clone(),
    };
    if !metadata.foldable && metadata.summary.is_none() {
        return None;
    }
    let json = serde_json::to_string(&metadata).ok()?;
    Some(format!(
        "{SECTION_METADATA_PREFIX}{json}{SECTION_METADATA_SUFFIX}"
    ))
}

fn parse_section_metadata_line(line: &str) -> Option<SectionMetadata> {
    let trimmed = line.trim();
    let json = trimmed
        .strip_prefix(SECTION_METADATA_PREFIX)?
        .strip_suffix(SECTION_METADATA_SUFFIX)?;
    serde_json::from_str(json).ok()
}

fn split_section_metadata(content: String) -> (SectionMetadata, String) {
    let mut metadata = SectionMetadata::default();
    let mut visible_lines = Vec::new();
    let mut parsing_metadata = true;

    for line in content.lines() {
        if parsing_metadata && let Some(parsed) = parse_section_metadata_line(line) {
            metadata = parsed;
            continue;
        }
        if parsing_metadata && line.trim().is_empty() {
            continue;
        }
        parsing_metadata = false;
        visible_lines.push(line);
    }

    (metadata, visible_lines.join("\n"))
}

/// If the document has unfilled streaming sections, return a reminder string
/// for the agent to continue filling them.
fn streaming_unfilled_reminder(doc: &CachedDocument) -> String {
    if let Some(next) = doc.streaming_next {
        let unfilled: Vec<String> = (next..doc.sections.len()).map(|i| i.to_string()).collect();
        if !unfilled.is_empty() {
            return format!(
                " Note: sections {} still need content. \
                 Continue filling them with update_document_section after answering.",
                unfilled.join(", ")
            );
        }
    }
    String::new()
}

fn next_streaming_section(doc: &CachedDocument) -> Option<(usize, String)> {
    doc.streaming_next.and_then(|next| {
        doc.sections
            .get(next)
            .map(|section| (next, section.heading.clone()))
    })
}

// @agent-facing
#[allow(dead_code)]
fn reading_view_display_mode_guidance() -> &'static str {
    "\
    READING VIEW DISPLAY MODES: The current turn may include a short state line like \
    'Current reading view display mode: Tui.' or \
    'Current reading view display mode: Browser.'.\n\
    - Tui: terminal-safe formatting only. Do NOT use LaTeX ($...$), Mermaid, or tables. \
    Use Unicode math symbols directly and keep formatting simple.\n\
    - Browser: full HTML rendering is available. Mermaid diagrams, tables, blockquotes, \
    and richer markdown are allowed. For every equation or math symbol, ALWAYS use the \
    <eq> convention instead of raw Unicode math or raw $...$ / $$...$$ delimiters. \
    Inline form: <eq latex=\"...\">spoken reading</eq>. Display form: \
    <eq latex=\"...\" display=\"block\">spoken reading</eq>. Self-closing form: \
    <eq latex=\"...\" speak=\"spoken reading\"/>. In Browser mode, the latex attribute \
    is rendered visually and the inner text or speak attribute is what gets read aloud. \
    The latex attribute must contain the raw LaTeX body only, with no $, $$, \\(, or \\[ delimiters. \
    The spoken reading should describe the equation the way a lecturer would say it aloud — \
    natural English, not literal LaTeX. Examples: \
    latex=\"x^2 + y^2 = z^2\" → \"x squared plus y squared equals z squared\", \
    latex=\"\\\\sum_{i=1}^n x_i\" → \"the sum of x i from 1 to n\", \
    latex=\"A \\\\cup B \\\\subseteq C\" → \"A union B is a subset of C\", \
    latex=\"\\\\alpha \\\\geq \\\\beta\" → \"alpha is at least beta\". \
    Do NOT read notation literally (\"angle bracket\", \"backslash langle\", \"left paren\"). \
    Describe meaning, not visual symbols."
}

#[allow(dead_code)]
const READING_VIEW_CONTENT_STYLE_GUIDANCE: &str = "Content style: write straight prose that continues the section's voice. \
     Do NOT use a Q:/A: format. If the answer would be unclear without context, \
     a short italic lead-in is fine (e.g. *On dropout:* ...), but skip it when \
     the meaning is obvious from placement. Don't overuse it. Do NOT prefix with \
     bold/italic topic lines like '**On the efficiency gains:**' or \
     '*Regarding caching:*' — just write the content directly.";

#[allow(dead_code)]
const READING_VIEW_SUMMARY_GUIDANCE: &str = "SUMMARY (required): Always set the `summary` parameter to a short descriptive \
     label of your answer (5-10 words), e.g. summary=\"Role of attention heads in GPT\". \
     This is used as a section label regardless of foldable.";

#[allow(dead_code)]
const READING_VIEW_FOLDABLE_GUIDANCE: &str = "FOLDABLE CONTENT: Set foldable=true for any inserted answer, explanation, \
     example, or deep dive — the user can collapse and re-expand it with `f`. \
     Only set foldable=false when the user explicitly asked for a permanent \
     rewrite of the original passage (i.e. update_document_section / a patch \
     that REPLACES old_text rather than appending after it). Default to \
     foldable=true unless you are sure the content is meant to overwrite the \
     original passage.";

#[allow(dead_code)]
const READING_VIEW_TOOL_CALL_ONLY_GUIDANCE: &str =
    "Do NOT output plain text; only tool calls are visible to the user.";

#[allow(dead_code)]
const READING_VIEW_REWRITE_BOUNDARY_GUIDANCE: &str =
    "Do NOT rewrite the entire section unless the user explicitly asks for a rewrite.";

pub fn reading_view_display_mode_state(mode: ReadingViewDisplayMode) -> &'static str {
    match mode {
        ReadingViewDisplayMode::Tui => {
            "Current reading view display mode: Tui. Follow the Tui formatting rules in the reading view tool descriptions."
        }
        ReadingViewDisplayMode::Browser => {
            "Current reading view display mode: Browser. Follow the Browser formatting rules in the reading view tool descriptions."
        }
    }
}

#[allow(dead_code)]
pub fn reading_view_selection_follow_up_guidance(mode: ReadingViewDisplayMode) -> String {
    let display_mode_state = reading_view_display_mode_state(mode);
    format!(
        "Preferred tool: patch_document_section on the exact selected text.\n\
         Use the selected text exactly as old_text.\n\
         For normal follow-ups, insert your answer after the selection.\n\
         For rewrite, simplify, or rephrase requests, replace the selection.\n\n\
         {display_mode_state}\n\n\
         {READING_VIEW_REWRITE_BOUNDARY_GUIDANCE}\n\
         {READING_VIEW_TOOL_CALL_ONLY_GUIDANCE}",
    )
}

#[allow(dead_code)]
pub fn reading_view_section_follow_up_guidance(mode: ReadingViewDisplayMode) -> String {
    let display_mode_state = reading_view_display_mode_state(mode);
    format!(
        "Preferred tool: patch_document_section near the most relevant passage.\n\
         Use old_text verbatim from the current section content.\n\
         If the user explicitly wants the entire section rewritten, use update_document_section.\n\
         If the question is about the section as a whole and no passage is relevant, \
         append_to_section is acceptable.\n\
         If the follow-up introduces a new topic, use add_document_section after this section.\n\n\
         {display_mode_state}\n\n\
         {READING_VIEW_REWRITE_BOUNDARY_GUIDANCE}\n\
         {READING_VIEW_TOOL_CALL_ONLY_GUIDANCE}",
    )
}

// ---------------------------------------------------------------------------
// Session-scoped cache
// ---------------------------------------------------------------------------

/// Opaque cache stored on `Session` so documents survive across `ToolRouter`
/// rebuilds within the same session.
///
/// The `ToolRouter` (and therefore the `DocumentReaderHandler`) is recreated on
/// every sampling-request roundtrip.  A per-instance cache would lose
/// previously presented documents, causing `append_to_section` /
/// `update_document_section` to fail with "No document with id …".
///
/// Storing the cache on `Session` mirrors how other cross-turn state (e.g.
/// `JsReplHandle`, `FileReferenceCache`) is managed.
/// Display mode for the reading view, set by the UI layer.
///
/// Controls formatting guidance given to the agent (e.g. whether to use
/// LaTeX or Unicode math symbols).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadingViewDisplayMode {
    /// Terminal (TUI) — no LaTeX/HTML rendering.
    #[default]
    Tui,
    /// Browser with full HTML/KaTeX/Mermaid rendering.
    #[allow(dead_code)]
    Browser,
}

pub struct DocumentCache {
    docs: Mutex<HashMap<String, CachedDocument>>,
    display_mode: Mutex<ReadingViewDisplayMode>,
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::with_display_mode(ReadingViewDisplayMode::default())
    }
}

impl DocumentCache {
    pub fn with_display_mode(mode: ReadingViewDisplayMode) -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
            display_mode: Mutex::new(mode),
        }
    }

    /// Set the display mode (called by the UI layer when the user changes
    /// reading view mode).
    #[allow(dead_code)]
    pub fn set_display_mode(&self, mode: ReadingViewDisplayMode) {
        if let Ok(mut m) = self.display_mode.lock() {
            *m = mode;
        }
    }

    /// Get the current display mode.
    pub fn display_mode(&self) -> ReadingViewDisplayMode {
        self.display_mode.lock().map(|m| *m).unwrap_or_default()
    }

    fn contains(&self, document_id: &str) -> bool {
        self.docs
            .lock()
            .map(|d| d.contains_key(document_id))
            .unwrap_or(false)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedDocument>> {
        self.docs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Restore a document into the cache from a replayed `PresentDocument`
    /// event.  Called during session resume so that subsequent tool calls
    /// (e.g. `present_reading_view(document_id=...)` without title/content)
    /// can serve the cached version instantly instead of forcing the agent
    /// to regenerate the document from scratch.
    #[allow(dead_code)]
    pub fn restore_document(&self, document_id: String, title: String, content: &str) {
        let sections = parse_sections(content);
        let mut cache = self.lock();
        cache.insert(
            document_id,
            CachedDocument {
                title,
                sections,
                streaming_next: None,
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadingViewConfigMode {
    Tui,
    Browser,
    Disabled,
}

fn parse_reading_view_config_mode(mode: Option<&str>) -> ReadingViewConfigMode {
    match mode.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("browser") => ReadingViewConfigMode::Browser,
        Some("disabled" | "off") => ReadingViewConfigMode::Disabled,
        _ => ReadingViewConfigMode::Tui,
    }
}

fn reading_view_config_mode_from_config(config: &Config) -> ReadingViewConfigMode {
    let configured_mode = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|t| t.get("reading_view"))
        .and_then(|v| {
            v.clone()
                .try_into::<crate::config::types::ReadingViewToml>()
                .ok()
        })
        .and_then(|rv| rv.mode);

    if configured_mode.is_some() {
        return parse_reading_view_config_mode(configured_mode.as_deref());
    }

    // Backward compatibility for configs written before `[reading_view].mode`
    // became the source of truth.
    if !config.features.enabled(Feature::ReadingView) {
        return ReadingViewConfigMode::Disabled;
    }

    ReadingViewConfigMode::Tui
}

#[allow(dead_code)]
pub(crate) fn reading_view_display_mode_from_config(config: &Config) -> ReadingViewDisplayMode {
    match reading_view_config_mode_from_config(config) {
        ReadingViewConfigMode::Browser => ReadingViewDisplayMode::Browser,
        _ => ReadingViewDisplayMode::Tui,
    }
}

pub(crate) fn reading_view_tools_enabled(config: &Config) -> bool {
    reading_view_config_mode_from_config(config) != ReadingViewConfigMode::Disabled
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub struct DocumentReaderHandler {
    pub tool_name: ToolName,
}

impl DocumentReaderHandler {
    pub fn new(tool_name: ToolName) -> Self {
        Self { tool_name }
    }
}

// ---------------------------------------------------------------------------
// Tool specs
// ---------------------------------------------------------------------------

// @agent-facing
#[allow(dead_code)]
pub static PRESENT_DOCUMENT_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::string(Some(
            "Unique slug identifying this document for targeted updates".to_string(),
        )),
    );
    properties.insert(
        "title".to_string(),
        JsonSchema::string(Some("Display title for the document".to_string())),
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::string(Some(
            "Full markdown content. Use ## headings to define sections.".to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "present_reading_view".to_string(),
        description: format!(
            "Present structured content in a sectioned reading view that the user can \
             navigate and ask follow-up questions about. Use this instead of inline \
             text whenever your response is a structured explanation with multiple \
             sections — paper walkthroughs, deep dives, research briefings, organized \
             reports, or any multi-section content with ## headings. \
             FOLLOW-UPS: When a reading view was previously presented and the user \
             asks ANY follow-up about the same topic (re-explain, simplify, go deeper, \
             different angle, etc.), ALWAYS use the reading view tools — either \
             update/append to the existing document or present a fresh one. Never \
             fall back to plain text for follow-ups on a topic that has a reading view. \
             Do NOT use this for short answers, confirmations, or conversational \
             replies unrelated to an active document. \
             IMPORTANT: Never mention 'KB', 'knowledge base', 'card', or \
             'card ID' in the title or content — the user cares about the subject \
             matter, not internal storage. Use the paper/topic name as the title \
             (e.g. 'Cosmos Policy Walkthrough', not 'KB Walkthrough: paper-cosmos-policy'). \
             Write as if you understand the material directly, not as if you are \
             reading from a database entry. \
             CITATION STYLE: Never use academic citation format like \
             'Smith et al. (2026)' or parenthetical years '(2025)'. Reading view \
             content is read aloud — use natural phrasing instead: 'the authors \
             showed...', 'researchers found...', 'the original transformer paper \
             introduced...'. Reference papers by title or description, not by \
             author-year citation. When the method name or paper title is short \
             and recognizable, prefer it over a generic phrase — say 'the GRPO \
             method showed' or 'the Attention paper demonstrated' rather than \
             just 'researchers showed'. Use 'researchers showed' only when \
             neither the method nor the paper name is short or memorable. \
             STREAMING STRUCTURE: For substantial multi-section reading views, \
             prefer a skeleton-first flow. First call present_reading_view with \
             the final title and all ## section headings, but leave the section \
             bodies empty. This opens the reading view immediately as an outline \
             or skeleton. Then immediately fill the sections in order with \
             update_document_section calls. Do not wait for user input between \
             those tool calls. For short or single-section reading views, \
             presenting the full content at once is fine. \
             FIGURES: To include figures from the paper, \
             call `crop_and_store_figure` with the page number and bounding box \
             coordinates for each figure you want to include. Then reference the \
             returned asset_path in your markdown as `![caption](asset_path)`. \
             You can also use ```mermaid code blocks for generated diagrams. \
             MATH: In Tui mode, use Unicode math symbols directly \
             (\u{03C0}, \u{03B8}, \u{03B1}, \u{2211}, \u{222B}, \u{2264}, \
             \u{2265}, \u{2192}, \u{00D7}, \u{207F}, \u{00B2}, \u{221A}, \
             \u{1D40}, etc.) instead of LaTeX. In Browser mode, ALWAYS encode \
             equations and math symbols with the <eq> convention instead of raw \
             Unicode math or raw $...$ / $$...$$ delimiters: inline \
             <eq latex=\"...\">spoken reading</eq>, display \
             <eq latex=\"...\" display=\"block\">spoken reading</eq>, or \
             self-closing <eq latex=\"...\" speak=\"spoken reading\"/>. In each \
             <eq>, the latex attribute is rendered visually and the inner text or \
             speak attribute is what gets read aloud. The latex attribute must \
             contain the raw LaTeX body only, with no $, $$, \\(, or \\[ delimiters. \
             The spoken reading should describe the equation the way a lecturer would \
             say it aloud — natural English, not literal LaTeX. Examples: \
             latex=\"x^2 + y^2 = z^2\" → \"x squared plus y squared equals z squared\", \
             latex=\"\\\\sum_{{i=1}}^n x_i\" → \"the sum of x i from 1 to n\", \
             latex=\"A \\\\cup B \\\\subseteq C\" → \"A union B is a subset of C\", \
             latex=\"\\\\alpha \\\\geq \\\\beta\" → \"alpha is at least beta\". \
             Do NOT read notation literally (\"angle bracket\", \"backslash langle\", \"left paren\"). \
             Describe meaning, not visual symbols. \
             NEVER use underscore subscript notation (W_Q, d_k) — write \
             subscripts as inline lowercase letters (Wq, Wk, Wv, dk) or use \
             Unicode sub/superscripts where available (\u{2080}\u{2081}\u{2082}, \
             \u{1D62}, \u{2096}, x\u{207F}, n\u{00B2}). \
             NEVER wrap math in backtick code spans or code blocks. \
             {mode_guidance} \
             CRITICAL — SILENCE AFTER PRESENTING: The reading view IS your \
             response. After calling this tool, do NOT output any additional text \
             in the chat — no summary, no recap, no 'here is what I found', no \
             restatement of the content. The user already sees the full document \
             in the reading view. Any text you add after this tool call will \
             appear as redundant duplication. The ONLY exception is a short \
             follow-up question to the user (e.g. 'Would you like me to go \
             deeper on any section?'). If you have no question, output nothing. \
             To re-display a previously presented \
             document (with all section updates intact), pass only the document_id.",
            mode_guidance = reading_view_display_mode_guidance(),
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["document_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

// @agent-facing
#[allow(dead_code)]
pub static UPDATE_DOCUMENT_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::string(Some(
            "The document to update (must match a previous present_reading_view call)".to_string(),
        )),
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::number(Some("0-based section index to replace".to_string())),
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::string(Some("New markdown content for the section".to_string())),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "update_document_section".to_string(),
        description: format!(
            "Replace the entire content of a section in a document being read by the user. \
             Use this to fill an empty section, or when the user explicitly asks to \
             rewrite/restructure/simplify the whole section. Prefer patch_document_section \
             for targeted edits. Use this only for whole-section rewrites, not for inserting \
             an answer after one passage. \
             {READING_VIEW_CONTENT_STYLE_GUIDANCE} \
             SILENCE AFTER UPDATING: The reading view already shows the updated \
             content. Do not repeat or summarize the changes in the chat. Only \
             add text if you have a follow-up question for the user.",
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "content".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

// @agent-facing
#[allow(dead_code)]
pub static APPEND_TO_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::string(Some(
            "The document to update (must match a previous present_reading_view call)".to_string(),
        )),
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::number(Some("0-based section index to append to".to_string())),
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::string(Some(
            "Markdown content to append at the end of the section".to_string(),
        )),
    );
    properties.insert(
        "foldable".to_string(),
        JsonSchema::boolean(Some(
            "When true, content appears in a collapsible region the user can fold/expand with `f`. \
             Set true for inserted answers, explanations, and examples. \
             Set false ONLY when the patch replaces the original passage with a permanent rewrite. \
             Default: true for append/patch when not specified."
                .to_string(),
        )),
    );
    properties.insert(
        "summary".to_string(),
        JsonSchema::string(Some(
            "Short descriptive label for this content (5-10 words). Always provide this. Used as fold title when collapsed."
                .to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "append_to_section".to_string(),
        description: format!(
            "Append content to the end of a section in a document currently being read. \
             Use this when adding information to a section without rewriting it entirely, \
             especially when the answer applies to the section as a whole and no single \
             passage is the right insertion point. \
             {READING_VIEW_CONTENT_STYLE_GUIDANCE} \
             {READING_VIEW_SUMMARY_GUIDANCE} \
             {READING_VIEW_FOLDABLE_GUIDANCE} \
             SILENCE AFTER APPENDING: The reading view already shows the appended \
             content. Do not repeat or summarize what you added in the chat. Only \
             add text if you have a follow-up question for the user.",
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "content".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

// @agent-facing
#[allow(dead_code)]
pub static PATCH_DOCUMENT_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::string(Some(
            "The document to update (must match a previous present_reading_view call)".to_string(),
        )),
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::number(Some("0-based section index to patch".to_string())),
    );
    properties.insert(
        "old_text".to_string(),
        JsonSchema::string(Some(
            "Exact text to find within the section content".to_string(),
        )),
    );
    properties.insert(
        "new_text".to_string(),
        JsonSchema::string(Some("Replacement text".to_string())),
    );
    properties.insert(
        "foldable".to_string(),
        JsonSchema::boolean(Some(
            "When true, content appears in a collapsible region the user can fold/expand with `f`. \
             Set true for inserted answers, explanations, and examples. \
             Set false ONLY when the patch replaces the original passage with a permanent rewrite. \
             Default: true for append/patch when not specified."
                .to_string(),
        )),
    );
    properties.insert(
        "summary".to_string(),
        JsonSchema::string(Some(
            "Short descriptive label for this content (5-10 words). Always provide this. Used as fold title when collapsed."
                .to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "patch_document_section".to_string(),
        description: format!(
            "Find and replace specific text within a section of a document currently \
             being read. Use this for targeted edits like fixing a sentence or updating \
             a specific paragraph without rewriting the entire section. To insert an \
             answer after an existing passage, set old_text to that exact passage and \
             set new_text to '<that same passage>\\n\\n<your answer>'. To rewrite a \
             passage, set old_text to the exact passage and set new_text to only the \
             rewritten replacement text, without including the old_text again. \
             {READING_VIEW_CONTENT_STYLE_GUIDANCE} \
             {READING_VIEW_SUMMARY_GUIDANCE} \
             {READING_VIEW_FOLDABLE_GUIDANCE} \
             SILENCE AFTER PATCHING: The reading view already shows the patched \
             content. Do not repeat or summarize the changes in the chat. Only \
             add text if you have a follow-up question for the user.",
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "old_text".to_string(),
                "new_text".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

#[allow(dead_code)]
pub static ADD_DOCUMENT_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::string(Some(
            "The document to update (must match a previous present_reading_view call)".to_string(),
        )),
    );
    properties.insert(
        "after_section_index".to_string(),
        JsonSchema::number(Some(
            "Insert the new section AFTER this 0-based index. Use -1 to insert at the beginning."
                .to_string(),
        )),
    );
    properties.insert(
        "heading".to_string(),
        JsonSchema::string(Some("The ## heading for the new section".to_string())),
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::string(Some("Markdown content for the section body".to_string())),
    );
    properties.insert(
        "foldable".to_string(),
        JsonSchema::boolean(Some(
            "When true, the new section starts collapsed. Use for supplementary \
             content. Default: false."
                .to_string(),
        )),
    );
    properties.insert(
        "summary".to_string(),
        JsonSchema::string(Some(
            "Short descriptive label for this section (5-10 words). Used as fold title when collapsed."
                .to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "add_document_section".to_string(),
        description: format!(
            "Add a new section to a document currently being read by the user. \
             Use this when a follow-up question introduces a new topic that deserves \
             its own section, rather than cramming new content into existing sections \
             via append. Do NOT use this to rewrite existing content \u{2014} use \
             update_document_section for that. \
             {READING_VIEW_CONTENT_STYLE_GUIDANCE} \
             {READING_VIEW_SUMMARY_GUIDANCE} \
             {READING_VIEW_FOLDABLE_GUIDANCE} \
             SILENCE AFTER ADDING: The reading view already shows the new section. \
             Do not repeat or summarize the content in the chat. Only add text if \
             you have a follow-up question for the user.",
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "document_id".to_string(),
                "after_section_index".to_string(),
                "heading".to_string(),
                "content".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

// ---------------------------------------------------------------------------
// ToolHandler impl
// ---------------------------------------------------------------------------

impl ToolHandler for DocumentReaderHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        self.tool_name.clone()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            tool_name,
            ..
        } = invocation;

        let is_subagent = matches!(turn.session_source, SessionSource::SubAgent(_));

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "document_reader handler received unsupported payload".to_string(),
                ));
            }
        };

        let doc_cache: &DocumentCache = &session.document_cache;
        if !reading_view_tools_enabled(turn.config.as_ref()) {
            return Err(FunctionCallError::RespondToModel(
                "Reading view tools are disabled for this session.".to_string(),
            ));
        }

        let content = match tool_name.name.as_str() {
            "present_reading_view" => {
                let args: PresentDocumentArgs = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse present_reading_view arguments: {e}"
                    ))
                })?;

                // Resolve title and content: use provided values, or fall back to cache.
                let (title, doc_content, _is_outline_only, _section_count): (
                    String,
                    String,
                    bool,
                    usize,
                ) = {
                    let mut cache = doc_cache.lock();
                    match (args.title, args.content) {
                        (Some(t), Some(c)) => {
                            // New document or full replacement — cache it.
                            let c = strip_citation_markers(&c);
                            let sections = parse_sections(&c);
                            let sec_count = sections.len();
                            // Detect outline-only: all headed sections have empty
                            // content and there are at least 2 sections.
                            let outline = sec_count > 1
                                && sections
                                    .iter()
                                    .all(|s| s.heading.is_empty() || s.content.trim().is_empty());
                            cache.insert(
                                args.document_id.clone(),
                                CachedDocument {
                                    title: t.clone(),
                                    sections,
                                    streaming_next: if outline { Some(0) } else { None },
                                },
                            );
                            (t, c, outline, sec_count)
                        }
                        _ => {
                            // Re-display from cache.
                            if let Some(cached) = cache.get(&args.document_id) {
                                (cached.title.clone(), cached.to_markdown(), false, 0_usize)
                            } else {
                                return Err(FunctionCallError::RespondToModel(format!(
                                    "No cached document with id \"{}\". \
                                     Provide title and content when presenting a document \
                                     for the first time.",
                                    args.document_id
                                )));
                            }
                        }
                    }
                };

                let doc_id = args.document_id;
                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: doc_id.clone(),
                                title: title.clone(),
                                content: doc_content.clone(),
                                is_reopen: false,
                            }),
                        )
                        .await;
                }
                if is_subagent {
                    // In a subagent context the TUI never receives the event
                    // (it only processes events from the active thread).
                    // Return the full document content inline so the parent
                    // agent receives it through the normal wait() path.
                    format!("# {title}\n\n{doc_content}")
                } else {
                    let display_mode_state =
                        reading_view_display_mode_state(doc_cache.display_mode());
                    if let Some(next_section) = doc_cache
                        .lock()
                        .get(&doc_id)
                        .and_then(next_streaming_section)
                    {
                        let (next_index, next_heading) = next_section;
                        format!(
                            "Document outline displayed in reading mode. The user can now see the \
                             skeleton immediately. NOW call update_document_section with \
                             section_index={next_index} ('{next_heading}') to fill the first \
                             section. After each update_document_section result, continue filling \
                             the next section until all sections are complete. Do not output plain \
                             text between these tool calls.\n\
                             \n\
                             {display_mode_state}"
                        )
                    } else {
                        format!(
                            "Document displayed in reading mode. The user can now navigate sections \
                             and ask follow-up questions. For ANY follow-up about this topic, use the \
                             reading view tools rather than a plain text answer.\n\
                             \n\
                             {display_mode_state}"
                        )
                    }
                }
            }
            "update_document_section" => {
                let mut args: UpdateDocumentSectionArgs = serde_json::from_str(&arguments)
                    .map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse update_document_section arguments: {e}"
                        ))
                    })?;
                args.content = strip_citation_markers(&args.content);
                args.content = strip_agent_authored_metadata(&args.content);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Bounds-check the section index before proceeding.
                {
                    let cache = doc_cache.lock();
                    if let Some(doc) = cache.get(&args.document_id)
                        && args.section_index >= doc.sections.len()
                    {
                        return Err(FunctionCallError::RespondToModel(format!(
                            "Section index {} is out of bounds. \
                                 Document has {} section(s) (valid indices: 0\u{2013}{}).",
                            args.section_index,
                            doc.sections.len(),
                            doc.sections.len().saturating_sub(1),
                        )));
                    }
                }

                // Mirror the update in the cache and advance streaming state.
                let (streaming_msg, reopen_payload, section_metadata) = {
                    let mut cache = doc_cache.lock();
                    let mut msg: Option<String> = None;
                    let mut metadata: (bool, Option<String>) = (false, None);
                    if let Some(doc) = cache.get_mut(&args.document_id) {
                        if let Some(section) = doc.sections.get_mut(args.section_index) {
                            if let Some(rest) = args.content.strip_prefix("## ") {
                                if let Some(nl) = rest.find('\n') {
                                    section.heading = rest[..nl].trim().to_string();
                                    section.content = rest[nl + 1..].to_string();
                                } else {
                                    section.heading = rest.trim().to_string();
                                    section.content = String::new();
                                }
                            } else {
                                section.content = args.content.clone();
                            }
                            metadata = (section.foldable, section.summary.clone());
                        }
                        // Advance streaming_next.
                        if let Some(next) = doc.streaming_next {
                            let new_next = next.max(args.section_index) + 1;
                            if new_next < doc.sections.len() {
                                doc.streaming_next = Some(new_next);
                                if let Some((next_index, next_heading)) =
                                    next_streaming_section(doc)
                                {
                                    msg = Some(format!(
                                        "Section {idx} updated. NOW call \
                                         update_document_section with \
                                         section_index={next_index} ('{next_heading}'). \
                                         Do not output text \u{2014} make the tool call \
                                         immediately.",
                                        idx = args.section_index,
                                    ));
                                }
                            } else {
                                doc.streaming_next = None;
                                msg = Some(
                                    "All sections filled. Wait for user interaction.".to_string(),
                                );
                            }
                        }
                    }
                    // When not streaming, capture payload to re-open the
                    // reading view in case the user closed it.
                    let reopen = if msg.is_none() {
                        cache
                            .get(&args.document_id)
                            .map(|doc| (doc.title.clone(), doc.to_markdown()))
                    } else {
                        None
                    };
                    (msg, reopen, metadata)
                };

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                                is_reopen: true,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::UpdateDocumentSection(UpdateDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                content: args.content,
                                foldable: section_metadata.0,
                                summary: section_metadata.1,
                            }),
                        )
                        .await;
                }
                streaming_msg.unwrap_or_else(|| {
                    "Section updated. The user can see the change immediately.".to_string()
                })
            }
            "append_to_section" => {
                let mut args: AppendToSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse append_to_section arguments: {e}"
                        ))
                    })?;
                args.content = strip_citation_markers(&args.content);
                args.content = strip_agent_authored_metadata(&args.content);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Bounds-check the section index before proceeding.
                {
                    let cache = doc_cache.lock();
                    if let Some(doc) = cache.get(&args.document_id)
                        && args.section_index >= doc.sections.len()
                    {
                        return Err(FunctionCallError::RespondToModel(format!(
                            "Section index {} is out of bounds. \
                                 Document has {} section(s) (valid indices: 0\u{2013}{}).",
                            args.section_index,
                            doc.sections.len(),
                            doc.sections.len().saturating_sub(1),
                        )));
                    }
                }

                // Capture the reopen payload from the PRE-append cache so a
                // closed reader re-opens with the pre-append content and the
                // subsequent AppendDocumentSection event applies the change
                // exactly once. If we built reopen_payload from the post-append
                // cache, a re-opened reader would receive the appended content
                // twice (once via PresentDocument, once via the append event).
                let (streaming_reminder, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let pre_reopen = cache
                        .get(&args.document_id)
                        .map(|doc| (doc.title.clone(), doc.to_markdown()));
                    let reminder = if let Some(doc) = cache.get_mut(&args.document_id) {
                        if let Some(section) = doc.sections.get_mut(args.section_index) {
                            if !section.content.is_empty() && !section.content.ends_with('\n') {
                                section.content.push('\n');
                            }
                            section.content.push_str(&args.content);
                            // Append defaults to foldable so the inserted answer
                            // is collapsible with `f` — see
                            // READING_VIEW_FOLDABLE_GUIDANCE.
                            section.foldable = args.foldable.unwrap_or(true);
                            section.summary = args.summary.clone();
                        }
                        streaming_unfilled_reminder(doc)
                    } else {
                        String::new()
                    };
                    let reopen = if reminder.is_empty() {
                        pre_reopen
                    } else {
                        None
                    };
                    (reminder, reopen)
                };

                let foldable = args.foldable.unwrap_or(true);
                let summary = args.summary;

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                                is_reopen: true,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::AppendDocumentSection(AppendDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                content: args.content,
                                foldable,
                                summary,
                            }),
                        )
                        .await;
                }
                format!(
                    "Content appended to section. The user can see the change immediately.\
                     {streaming_reminder}"
                )
            }
            "add_document_section" => {
                let mut args: AddDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse add_document_section arguments: {e}"
                        ))
                    })?;
                args.content = strip_citation_markers(&args.content);
                args.content = strip_agent_authored_metadata(&args.content);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Bounds-check after_section_index: must be -1..len-1.
                {
                    let cache = doc_cache.lock();
                    if let Some(doc) = cache.get(&args.document_id) {
                        let max_idx = doc.sections.len() as i32 - 1;
                        if args.after_section_index < -1 || args.after_section_index > max_idx {
                            return Err(FunctionCallError::RespondToModel(format!(
                                "after_section_index {} is out of bounds. \
                                 Document has {} section(s) (valid range: -1\u{2013}{max_idx}).",
                                args.after_section_index,
                                doc.sections.len(),
                            )));
                        }
                    }
                }

                // Capture the reopen payload from the PRE-insert cache so an
                // already-open reader applies the new section exactly once via
                // the AddDocumentSection event below. If we built reopen_payload
                // from the post-insert cache, an already-open reader would
                // receive the new section twice (once via PresentDocument
                // re-open, once via the AddDocumentSection event).
                let (insert_pos, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let reopen = cache
                        .get(&args.document_id)
                        .map(|doc| (doc.title.clone(), doc.to_markdown()));
                    let pos = if let Some(doc) = cache.get_mut(&args.document_id) {
                        let insert_at = (args.after_section_index + 1) as usize;
                        doc.sections.insert(
                            insert_at,
                            CachedSection {
                                heading: args.heading.clone(),
                                content: args.content.clone(),
                                foldable: args.foldable.unwrap_or(false),
                                summary: args.summary.clone(),
                            },
                        );
                        Some(insert_at)
                    } else {
                        None
                    };
                    (pos, reopen)
                };

                let foldable = args.foldable.unwrap_or(false);
                let summary = args.summary;
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                                is_reopen: true,
                            }),
                        )
                        .await;
                }

                if !is_subagent && let Some(_pos) = insert_pos {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::AddDocumentSection(AddDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                after_section_index: args.after_section_index,
                                heading: args.heading,
                                content: args.content,
                                foldable,
                                summary,
                            }),
                        )
                        .await;
                }
                "New section added. The user can see the change immediately.".to_string()
            }
            "patch_document_section" => {
                let mut args: PatchDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse patch_document_section arguments: {e}"
                        ))
                    })?;
                args.new_text = strip_citation_markers(&args.new_text);
                args.new_text = strip_agent_authored_metadata(&args.new_text);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Bounds-check section index and verify old_text exists.
                {
                    let cache = doc_cache.lock();
                    if let Some(doc) = cache.get(&args.document_id) {
                        if args.section_index >= doc.sections.len() {
                            return Err(FunctionCallError::RespondToModel(format!(
                                "Section index {} is out of bounds. \
                                 Document has {} section(s) (valid indices: 0\u{2013}{}).",
                                args.section_index,
                                doc.sections.len(),
                                doc.sections.len().saturating_sub(1),
                            )));
                        }
                        if let Some(section) = doc.sections.get(args.section_index)
                            && !section.content.contains(&args.old_text)
                        {
                            let preview: String = section.content.chars().take(120).collect();
                            return Err(FunctionCallError::RespondToModel(format!(
                                "old_text not found in section {}. \
                                     Section content starts with: \"{preview}\u{2026}\"",
                                args.section_index,
                            )));
                        }
                    }
                }

                // Capture the reopen payload from the PRE-patch cache so the
                // frontend can re-open a closed reader and then apply the
                // subsequent PatchDocumentSection event exactly once. If we
                // built reopen_payload from the post-patch cache, an already-
                // open reader would apply the change twice (once via the
                // PresentDocument re-open, once via the patch event).
                let (streaming_reminder, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let reminder = cache
                        .get(&args.document_id)
                        .map(streaming_unfilled_reminder)
                        .unwrap_or_default();
                    let reopen = if reminder.is_empty() {
                        cache
                            .get(&args.document_id)
                            .map(|doc| (doc.title.clone(), doc.to_markdown()))
                    } else {
                        None
                    };
                    // Now mirror the patch in the cache so subsequent reads
                    // (e.g. for streaming reminders, future reopens) see it.
                    if let Some(doc) = cache.get_mut(&args.document_id)
                        && let Some(section) = doc.sections.get_mut(args.section_index)
                        && section.content.contains(&args.old_text)
                    {
                        section.content =
                            section.content.replacen(&args.old_text, &args.new_text, 1);
                        // Patch defaults to foldable so an inserted answer is
                        // collapsible with `f` — see READING_VIEW_FOLDABLE_GUIDANCE.
                        section.foldable = args.foldable.unwrap_or(true);
                        section.summary = args.summary.clone();
                    }
                    (reminder, reopen)
                };

                let foldable = args.foldable.unwrap_or(true);
                let summary = args.summary;

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                                is_reopen: true,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PatchDocumentSection(PatchDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                old_text: args.old_text,
                                new_text: args.new_text,
                                foldable,
                                summary,
                            }),
                        )
                        .await;
                }
                format!(
                    "Section patched. The user can see the change immediately.\
                     {streaming_reminder}"
                )
            }
            other => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "document_reader handler received unknown tool: {other}"
                )));
            }
        };

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_citation_markers() {
        assert_eq!(strip_citation_markers("hello citeturn0view0"), "hello");
        assert_eq!(
            strip_citation_markers("quality. citeturn1view10turn7view0"),
            "quality."
        );
        assert_eq!(
            strip_citation_markers("text citeturn5search0 more"),
            "text more"
        );
        assert_eq!(
            strip_citation_markers("mixed citeturn7view0turn5search0turn1view2"),
            "mixed"
        );
        assert_eq!(strip_citation_markers("no markers"), "no markers");
    }

    #[test]
    fn next_streaming_section_returns_next_index_and_heading() {
        let doc = CachedDocument {
            title: "Doc".to_string(),
            sections: vec![
                CachedSection {
                    heading: "Overview".to_string(),
                    content: String::new(),
                    foldable: false,
                    summary: None,
                },
                CachedSection {
                    heading: "Method".to_string(),
                    content: String::new(),
                    foldable: false,
                    summary: None,
                },
            ],
            streaming_next: Some(1),
        };

        assert_eq!(
            next_streaming_section(&doc),
            Some((1, "Method".to_string()))
        );
    }

    #[test]
    fn parse_reading_view_config_mode_accepts_disabled_and_off() {
        assert_eq!(
            parse_reading_view_config_mode(Some("browser")),
            ReadingViewConfigMode::Browser
        );
        assert_eq!(
            parse_reading_view_config_mode(Some("disabled")),
            ReadingViewConfigMode::Disabled
        );
        assert_eq!(
            parse_reading_view_config_mode(Some("off")),
            ReadingViewConfigMode::Disabled
        );
        assert_eq!(
            parse_reading_view_config_mode(Some("tui")),
            ReadingViewConfigMode::Tui
        );
    }

    #[tokio::test]
    async fn reading_view_mode_overrides_legacy_feature_flag() -> std::io::Result<()> {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(codex_config::CONFIG_TOML_FILE),
            r#"[reading_view]
mode = "disabled"

[features]
reading_view = true
"#,
        )?;

        let config = crate::config::ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .harness_overrides(crate::config::ConfigOverrides {
                cwd: Some(tmp.path().to_path_buf()),
                ..Default::default()
            })
            .build()
            .await?;

        assert_eq!(
            reading_view_config_mode_from_config(&config),
            ReadingViewConfigMode::Disabled
        );
        assert!(!reading_view_tools_enabled(&config));
        assert_eq!(
            reading_view_display_mode_from_config(&config),
            ReadingViewDisplayMode::Tui
        );

        Ok(())
    }

    #[test]
    fn present_reading_view_tool_description_mentions_skeleton_first_flow() {
        let ToolSpec::Function(tool) = &*PRESENT_DOCUMENT_TOOL else {
            panic!("present_reading_view should be a function tool");
        };

        assert!(
            tool.description.contains("prefer a skeleton-first flow"),
            "tool description should instruct the model to present an outline first for substantial documents"
        );
        assert!(
            tool.description.contains(
                "Then immediately fill the sections in order with update_document_section calls"
            ),
            "tool description should explain the follow-up section fill sequence"
        );
    }
}
