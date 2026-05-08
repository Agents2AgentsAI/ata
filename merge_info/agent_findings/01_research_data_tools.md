# Fork-vs-Upstream Divergence Analysis: Research & Data Tools

**Status**: Comprehensive analysis of research tools, data tools, reading view, document reader, figure extraction, and related knowledge-base skills in the ATA fork vs. upstream Codex `rust-v0.129.0`.

**Date**: 2025-05-07

---

## Summary

The ATA fork has **extensive local-only features** in the research and data tools space. Upstream (`rust-v0.129.0`) contains **none** of these implementations. All discoveries listed below are fork-only and must be preserved during merge.

- **Local-only crates**: `codex-research-tools`, `codex-data-tools`, `reading-view-server`, `treesitter`
- **Local-only tool handlers**: `research`, `data`, `document_reader`, `crop_figure`
- **Local-only skills**: 13 research skills, KB skill, all in `codex-rs/skills/src/assets/research/`
- **Local-only protocol features**: Document reader events in `codex-protocol`
- **Merge strategy**: Keep all local implementations; no upstream adoption needed.

---

## Local-Only Features (Fork Only)

### 1. Research Tools Crate (`codex-research-tools`)

**Name**: Research Tools Toolkit

**Description**: A comprehensive Rust library providing unified access to multiple academic and research data sources. Enables agents to search papers, manage citations, access datasets, monitor hacker news, search patents, and manage Zotero libraries.

**Implementation Summary**:
- **Crate location**: `/codex-rs/codex-research-tools/`
- **Core modules**:
  - `src/lib.rs` — `ResearchToolkit` main struct orchestrating all tools
  - `src/config.rs` — Configuration for API credentials, timeouts, rate limits
  - `src/clients/` — Client implementations for each provider:
    - `arxiv.rs` — arXiv paper search and metadata
    - `openalex.rs` — OpenAlex academic search
    - `semantic_scholar.rs` — Semantic Scholar integration
    - `zotero.rs` — Zotero client for library management
    - `hackernews.rs` — HackerNews API client
    - `patents.rs` — Patent search (USPTO/EPO via OAuth)
    - `epo_auth.rs` — EPO authentication manager
    - `github.rs` — GitHub repository analysis
  - `src/tools/` — High-level tool operations:
    - `paper_search.rs` — Multi-source paper discovery
    - `zotero.rs` — Comprehensive Zotero operations (advanced search, citations, collections, mutations)
    - `zotero/*.rs` — Zotero submodules (advanced_search, budget, content_collector, core, document_resolution, grep, item_endpoints, match_engine, query_endpoints, search_notes, write_endpoints, tests)
    - `hackernews.rs` — HackerNews thread retrieval and searching
    - `patents.rs` — Patent search and retrieval
    - `repo_analysis.rs` — GitHub repository analysis
  - `src/types.rs` — Request/response types for all tools
  - `src/http_client.rs`, `src/cache.rs`, `src/rate_limiter.rs` — Shared infrastructure
  - `src/tool_specs.rs` — JSON schema specs for model tool calling

**Status vs Upstream**: **Local-only** — not present in `rust-v0.129.0` at all.

**Merge Plan**: Keep all code as-is. No upstream equivalent to adopt or merge. Ensure `codex-research-tools` remains a workspace crate and is always included in builds. Verify `codex-rs/core/Cargo.toml` has both `codex-research-tools` (non-optional) and `codex-data-tools` (optional) dependencies.

---

### 2. Data Tools Crate (`codex-data-tools`)

**Name**: Data Tools Toolkit

**Description**: A Rust library for discovering, searching, and downloading datasets from Kaggle and Hugging Face. Provides agents with dataset search, metadata retrieval, file listing, and download capabilities.

**Implementation Summary**:
- **Crate location**: `/codex-rs/codex-data-tools/`
- **Core modules**:
  - `src/lib.rs` — `DataToolkit` main struct
  - `src/config.rs` — Configuration for Kaggle/HF API credentials
  - `src/clients/`:
    - `kaggle.rs` — Kaggle datasets and competitions API
    - `huggingface.rs` — Hugging Face datasets API
  - `src/tools/`:
    - `dataset_ops.rs` — Dataset search, get, list files, download operations
  - `src/types.rs` — Request/response types
  - `src/http_client.rs`, `src/cache.rs`, `src/rate_limiter.rs` — Shared infrastructure
  - `src/tool_specs.rs` — JSON schema specs

**Status vs Upstream**: **Local-only** — not present in `rust-v0.129.0`.

**Merge Plan**: Keep all code. This is a gated feature (optional in `Cargo.toml` under `data` feature flag). Ensure feature is properly enabled during build if data tools are to be active.

---

### 3. Research Tool Handler (`core/src/tools/handlers/research.rs`)

**Name**: Research Bridge Handler

**Description**: Tool handler that bridges the core Codex tool execution layer with the `codex-research-tools` library. Dispatches research tool calls from the agent to appropriate research toolkit methods and serializes results.

**Implementation Summary**:
- **File**: `codex-rs/core/src/tools/handlers/research.rs`
- **Key components**:
  - `ResearchBridgeHandler` struct — implements `ToolHandler` trait
  - `execute_native_tool()` — panic-safe wrapper for tool execution
  - `dispatch_tool_call()` — router dispatching tool names to methods:
    - `paper_search`, `paper_citations`, `paper_references`, `paper_get`, `paper_recommendations` — paper discovery
    - `zotero_*` methods (search, create, update, annotations, collections, etc.) — Zotero operations
    - `hackernews_search`, `hackernews_get_thread` — HN tools
    - `patent_search`, `patent_get` — Patent tools
    - `github_analyze_repo` — Repo analysis
  - Error handling and response serialization

**Status vs Upstream**: **Local-only** — upstream has no `research.rs` handler.

**Merge Plan**: Keep handler intact. Ensure it's registered in `codex-rs/core/src/tools/handlers/mod.rs` and exported as part of the handler suite.

---

### 4. Data Tool Handler (`core/src/tools/handlers/data.rs`)

**Name**: Data Bridge Handler

**Description**: Tool handler bridging core execution with `codex-data-tools` library. Routes dataset tool calls and manages dataset operations.

**Implementation Summary**:
- **File**: `codex-rs/core/src/tools/handlers/data.rs`
- **Key components**:
  - `DataBridgeHandler` struct — implements `ToolHandler` trait
  - `dispatch_tool_call()` — routes:
    - `dataset_search`, `dataset_get`, `dataset_list_files`, `dataset_download` — generic dataset operations
    - `hf_dataset_info` — Hugging Face dataset metadata
    - `kaggle_dataset_info`, `kaggle_competitions`, `kaggle_competition_list_files`, `kaggle_competition_download` — Kaggle-specific operations
  - Secrets management for API credentials

**Status vs Upstream**: **Local-only** — no `data.rs` in upstream.

**Merge Plan**: Keep handler. Register in `mod.rs`. Gated behind the `data` feature flag in `Cargo.toml`.

---

### 5. Reading View Server (`reading-view-server/`)

**Name**: Reading View Server (Living Reading View HTTP Server)

**Description**: A lightweight Axum-based HTTP + WebSocket server that serves an embedded HTML template (Living Reading View) and streams document events to browser clients. Enables rich, interactive rendering of research documents with sections, karaoke highlights, follow-up questions, and bidirectional communication.

**Implementation Summary**:
- **Crate location**: `/codex-rs/reading-view-server/`
- **Core functionality**:
  - `src/lib.rs` — main server:
    - `ReadingViewServer` struct — manages broadcast channel, event buffer, WebSocket clients
    - `start()` — spawns HTTP listener on random port, returns server handle
    - `send_event()` — broadcasts JSON events to all connected clients
    - Embedded HTML template via `include_str!("assets/LivingReadingView.html")`
    - Static file serving from optional `assets_root`
    - Callback channel for browser messages (follow-ups, read-aloud requests)
  - `src/assets/LivingReadingView.html` — embedded client-side template
- **Key features**:
  - Stateless event replay for late-joining WebSocket clients
  - Multi-client broadcast with configurable buffer capacity
  - Bidirectional WebSocket messaging
  - Optional integration with browser file serving

**Status vs Upstream**: **Local-only** — no `reading-view-server` crate in upstream.

**Merge Plan**: Keep entire crate. Ensure it's included in workspace `Cargo.toml` and properly linked from core/TUI modules that use it. No upstream conflicts.

---

### 6. Document Reader Tool Handler (`core/src/tools/handlers/document_reader.rs`)

**Name**: Document Reader (Living Reading View Integration)

**Description**: Tool handler providing agent-facing commands to build, update, and stream structured documents to the Living Reading View. Implements `present_document`, `add_document_section`, `append_to_section`, `update_document_section`, `patch_document_section` commands. Manages document state caching and streaming indicators.

**Implementation Summary**:
- **File**: `codex-rs/core/src/tools/handlers/document_reader.rs`
- **Key components**:
  - `DocumentReaderHandler` struct
  - `CachedDocument` — in-memory document state with sections, streaming metadata
  - `present_document()` — initializes reading view with title + section structure
  - `add_document_section()` — appends new section
  - `append_to_section()` — streams content to existing section
  - `update_document_section()` — replaces section content entirely
  - `patch_document_section()` — surgical patch to section (useful for foldable content)
  - `strip_citation_markers()` — removes internal citation artifacts from output
  - `parse_sections()` — markdown-to-sections parser (splits on `## ` headings)
  - Streaming state tracking (unfilled section reminders for continued agent work)
  - TUI vs. Browser mode detection and formatting guidance

**Status vs Upstream**: **Local-only** — upstream has no `document_reader.rs`.

**Merge Plan**: Keep handler. Register in `mod.rs`. Ensure `codex-protocol` has corresponding `document_reader` event types (see protocol section).

---

### 7. Crop Figure Tool Handler (`core/src/tools/handlers/crop_figure.rs`)

**Name**: Figure Extraction (PDF Page Cropping)

**Description**: Tool handler for extracting and cropping figures from PDF pages. Uses pdfium-render to render PDF pages at high resolution and extract rectangular regions as PNG images. Integrates with document reading view for figure inline display.

**Implementation Summary**:
- **File**: `codex-rs/core/src/tools/handlers/crop_figure.rs`
- **Key components**:
  - `CropFigureHandler` struct
  - `render_pdf_page()` — uses pdfium-render with 150 DPI rendering, searches for pdfium lib in:
    - `~/.ata/lib/` (user codex home)
    - Executable directory
    - System library paths
  - `find_cached_pdf()` — locates cached PDF by URL in codex cache dir
  - Crop coordinates (x, y, w, h) mapping to page dimensions
  - PNG output with SHA256 content hashing for cache busting
  - Caption and description metadata storage
  - Error messages guiding users to download libpdfium if missing

**Status vs Upstream**: **Local-only** — upstream has no `crop_figure.rs`.

**Merge Plan**: Keep handler. Register in `mod.rs`. Ensure pdfium-render is in workspace dependencies. Note: requires libpdfium binary availability at runtime.

---

### 8. TreeSitter Crate (`treesitter/`)

**Name**: TreeSitter Code Parser Integration

**Description**: Rust bindings and utilities for tree-sitter-based code parsing. Provides AST traversal, symbol extraction, and code navigation capabilities for code intelligence features.

**Implementation Summary**:
- **Crate location**: `/codex-rs/treesitter/`
- **Workspace integration**: Optional crate, referenced by core (gated on `treesitter` feature)
- **Likely contents** (based on directory listing): Tree-sitter language bindings, AST utilities, symbol extraction

**Status vs Upstream**: **Local-only** — not in `rust-v0.129.0`.

**Merge Plan**: Keep entire crate as-is. No upstream equivalent. Ensure workspace feature gates are correct.

---

### 9. Research Tool Spec Registration

**Name**: Research Tool Specification Registry

**Description**: JSON schema definitions for all research tools, enabling the model to understand tool signatures, parameters, and return types. Auto-generated from `codex-research-tools/src/tool_specs.rs`.

**Implementation Summary**:
- **File**: `codex-rs/codex-research-tools/src/tool_specs.rs`
- **Approach**: Defines `ConfigSchema` and `ModelDefinition` types; tool specs serialized to JSON for model consumption
- **Covers all tools**: paper_search, zotero_*, hackernews_*, patents_*, github_*, paper_recommendations, etc.

**Status vs Upstream**: **Local-only** — no research tools in upstream means no upstream specs.

**Merge Plan**: Keep all schema definitions. Ensure specs are properly loaded and registered during core initialization.

---

### 10. Data Tool Spec Registration

**Name**: Data Tool Specification Registry

**Description**: JSON schema definitions for data tools (dataset search, Kaggle, Hugging Face).

**Implementation Summary**:
- **File**: `codex-rs/codex-data-tools/src/tool_specs.rs`
- **Covers**: dataset_search, dataset_get, dataset_list_files, dataset_download, hf_dataset_info, kaggle_dataset_info, kaggle_competitions, kaggle_competition_list_files, kaggle_competition_download

**Status vs Upstream**: **Local-only**.

**Merge Plan**: Keep specs. Register appropriately (gated by `data` feature flag if applicable).

---

## Research and Knowledge-Base Skills (Local-Only)

All research skills are located in `codex-rs/skills/src/assets/research/` and are **local-only** (not in upstream). Upstream has only generic sample skills in `samples/`.

### Skill List

| Skill Name | Description | SKILL.md Present? |
|------------|-------------|-------------------|
| **zotero** | Zotero library management via `ata zotero` CLI namespace | ✓ |
| **paper-discovery** | Discover and rank papers for research topics | ✓ |
| **paper-synthesis** | Synthesize insights from multiple papers | ✓ |
| **paper-synthesizer** | (variant/sibling of paper-synthesis) | ✓ |
| **cross-paper-report** | Generate comparative analysis across papers | ✓ |
| **paper-discoverer** | (variant/sibling of paper-discovery) | ✓ |
| **research-briefing** | Create research briefings | ✓ |
| **hn-synthesis** | Synthesize HackerNews discussions | ✓ |
| **hn-synthesizer** | (variant of hn-synthesis) | ✓ |
| **hn-discoverer** | Discover and curate HackerNews stories | ✓ |
| **conversation-report** | Generate reports from conversations | ✓ |
| **kb** | Knowledge base card operations | ✓ |

**Status vs Upstream**: All 12+ research skills are **local-only**.

**Merge Plan**: Keep all skill assets. These are pure data/documentation (YAML/MD) and do not conflict with upstream. Upstream skills (`samples/*`) will coexist with research skills in the merged build.

---

## Protocol Extensions (Document Reader Events)

**Name**: Document Reader Protocol Events

**Description**: Event types in `codex-protocol` for document reader:
- `PresentDocumentEvent` / `PresentDocumentArgs` — initialize reading view
- `AddDocumentSectionEvent` / `AddDocumentSectionArgs` — add section
- `AppendDocumentSectionEvent` / `AppendToSectionArgs` — stream content
- `UpdateDocumentSectionEvent` / `UpdateDocumentSectionArgs` — replace section
- `PatchDocumentSectionEvent` / `PatchDocumentSectionArgs` — patch section

**Status vs Upstream**: **Local-only** — these protocol extensions are not in upstream.

**Merge Plan**: Verify these are defined in `codex-rs/protocol/src/` and that `codex-rs/core/src/tools/handlers/document_reader.rs` uses them correctly. Ensure protocol version/schema is compatible.

---

## Configuration & Secrets Integration

**Names**: Research/Data configuration and secrets management

**Description**: Both research and data tools integrate with `codex-secrets` for API credential management and `codex-config` for tool configuration (rate limits, timeouts, cache settings).

**Implementation Summary**:
- **Research config**: `codex-research-tools/src/config.rs` — loads from environment/config, manages rate limits, request timeouts
- **Data config**: `codex-data-tools/src/config.rs` — similar structure
- **Secrets**: Both use `codex-secrets` backend for API keys (Zotero API key, Kaggle token, HF token, EPO credentials, etc.)
- **Environment**: Tools auto-detect credentials from environment variables or stored secrets

**Status vs Upstream**: **Local-only** (no upstream implementations to merge with).

**Merge Plan**: Keep all configuration. Ensure environment variable names and secrets backend integration are documented and match deployment expectations.

---

## Testing Infrastructure

**Description**: Test utilities and example scripts for validating research and data tools.

**Implementation Summary**:
- **Research tools**: `codex-rs/codex-research-tools/src/tools/test_helpers.rs` — test fixtures
- **Data tools**: `codex-rs/codex-data-tools/examples/` — example usage scripts:
  - `test_kaggle.rs` — Kaggle integration test
  - `test_search.rs` — Dataset search example
  - `test_mnist.rs` — MNIST dataset example
- **CLI tests**: `codex-rs/cli/tests/zotero_search_commands.rs` — Zotero CLI integration tests

**Status vs Upstream**: **Local-only**.

**Merge Plan**: Keep all tests and examples. These are essential for validating research/data functionality post-merge.

---

## Build & Integration Points

### Cargo Workspace Changes

**File**: `codex-rs/Cargo.toml`

**Required setup**:
```toml
[workspace.members]
# ... existing members ...
"codex-research-tools",
"codex-data-tools",
"reading-view-server",
"treesitter",
```

**Status**: Verify these crates are listed in workspace members.

### Core Dependency Integration

**File**: `codex-rs/core/Cargo.toml`

**Required**:
```toml
[dependencies]
codex-research-tools = { workspace = true }  # non-optional
codex-data-tools = { workspace = true, optional = true }  # gated on 'data' feature
# ... (reading-view-server used by TUI/core)

[features]
research = []  # enable research tools
data = ["dep:codex-data-tools"]  # enable data tools
```

**Status**: Verify dependencies and feature gates are present.

### Handler Registration

**File**: `codex-rs/core/src/tools/handlers/mod.rs`

**Required exports**:
```rust
pub(crate) mod research;
pub(crate) mod data;
pub(crate) mod document_reader;
pub(crate) mod crop_figure;

pub use research::ResearchBridgeHandler;
pub use data::DataBridgeHandler;
pub use document_reader::DocumentReaderHandler;
pub use crop_figure::CropFigureHandler;
```

**Status**: Verify these are exported from the module and registered in the tool router.

---

## Merge Conflicts & Risks

### Low Risk (Minimal Integration)
- Crates are self-contained and new — no upstream files to conflict with
- Feature gates isolate optional tools (data, treesitter)
- Skills are pure data (YAML/MD) — no code conflicts

### Medium Risk (Integration Points)
- **Tool router**: Ensure `research`, `data`, `document_reader`, `crop_figure` handlers are properly registered
- **Specs loading**: Verify tool specs from new crates are loaded into the registry
- **Protocol**: Ensure document reader event types are properly serialized/deserialized

### Mitigation
1. After merge, build with all features enabled: `cargo build --features "data,treesitter,research"`
2. Run tool registry tests to verify all handlers are discoverable
3. Smoke-test each research tool (paper_search, zotero_search, hackernews_search)
4. Verify specs are loaded and model can see tool signatures

---

## Summary: Merge Plan & Actions

| Item | Action | Owner | Verification |
|------|--------|-------|--------------|
| codex-research-tools crate | Keep all code | merge-upstream | Build succeeds, specs load |
| codex-data-tools crate | Keep all code | merge-upstream | Build with `--features data` |
| reading-view-server crate | Keep all code | merge-upstream | No conflicts with TUI |
| treesitter crate | Keep all code | merge-upstream | Build with `--features treesitter` |
| research.rs handler | Keep all code | merge-upstream | Verify registration in mod.rs |
| data.rs handler | Keep all code | merge-upstream | Verify registration, feature gate |
| document_reader.rs handler | Keep all code | merge-upstream | Verify protocol integration |
| crop_figure.rs handler | Keep all code | merge-upstream | Verify pdfium availability |
| research skills | Keep all assets | merge-upstream | No upstream conflict (upstream has samples/) |
| KB skill | Keep all assets | merge-upstream | No upstream conflict |
| Workspace Cargo.toml | Verify members listed | merge-upstream | cargo check succeeds |
| core/Cargo.toml | Verify dependencies & features | merge-upstream | cargo check succeeds |
| mod.rs exports | Verify handlers exported | merge-upstream | Tool discovery works |

---

## Cross-References

### TUI Integration
- TUI reading view rendering likely uses `reading-view-server` — verify WebSocket integration in `codex-rs/tui/`
- Document reader state management — ensure TUI can stream sections and handle streaming indicators

### Skill Integration
- Skills dispatcher should find research skills under `skills/src/assets/research/`
- KB skill provides file-based storage — verify integration with file tools

### Protocol & Exec Integration
- `exec/` or `exec-server/` likely invoices these tools — ensure tool router includes research/data handlers
- Document reader events flow through protocol layer — verify event marshalling

---

## Configuration Notes

### Runtime Environment Variables
- `CODEX_RESEARCH_*` — research tool configuration
- `CODEX_DATA_*` — data tool configuration
- `CODEX_KB_PATH` — knowledge base directory (defaults to `~/.ata/knowledge-base`)

### API Credentials (Secrets)
Tools expect credentials to be managed via `codex-secrets`:
- Zotero API key
- Kaggle API credentials
- Hugging Face API token
- EPO patent API (OAuth)

---

## Timeline & Milestones

1. **Pre-merge**: Verify all crates and handlers compile locally with `main`
2. **During merge**: Resolve any upstream conflicts in shared files (none expected)
3. **Post-merge**: 
   - Full build with all features
   - Tool discovery validation
   - Smoke tests for each research/data tool
   - TUI reading view regression tests

---

