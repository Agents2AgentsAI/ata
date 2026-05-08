## Research/Data Tools Analysis (Agent 3)

All five focus crates are **entirely local-only** — they do not exist in `rust-v0.129.0`. Confirmed via `git show rust-v0.129.0:codex-rs/`. Likewise the corresponding handlers in `core/src/tools/handlers/` (`research.rs`, `data.rs`, `document_reader.rs`, `crop_figure.rs`, `code_intel.rs`, `attach_url_files.rs`) and the `core/src/research/` and `core/src/data/` modules are local-only.

### 1. Paper Search & Citation Graph
- **Type**: Local-only
- **Description**: Multi-source academic paper search across Semantic Scholar, arXiv, OpenAlex; resolve a paper by DOI/arXiv/S2 ID; traverse the citation/reference graph; get content-based recommendations.
- **Implementation**: Crate `codex-research-tools`. Clients in `src/clients/{semantic_scholar,arxiv,openalex}.rs`. Tool layer in `src/tools/paper_search.rs`. Tools (native + MCP names): `paper_search`/`search_papers`, `paper_get`/`get_paper`, `paper_citations`/`get_citations`, `paper_references`/`get_references`, `paper_recommendations`/`get_recommendations`. `ResearchToolkit` (`src/lib.rs`) is the entry point. Wired into core via `core/src/tools/handlers/research.rs` (`ResearchBridgeHandler`) and gated by `Feature::ResearchPaperSearch`.
- **Merge plan**: Re-add the workspace member entries (`codex-rs/Cargo.toml` lines `71`, `114-136`) and `core/Cargo.toml` `codex-research-tools` dep.

### 2. Zotero Library Integration (Read + Write)
- **Type**: Local-only
- **Description**: Search, browse, grep, and mutate a user's Zotero library (local API at `localhost:23119` or remote api.zotero.org). Includes notes/annotations/full-text/attachments retrieval, advanced search, tag search, collection CRUD, item CRUD, attachment-link creation, and citation rendering.
- **Implementation**: `codex-research-tools/src/clients/zotero.rs` + `src/tools/zotero/` (15 submodules). Tool surface (~25 ops): `zotero_search`, `zotero_get_recent`, `zotero_get_tags`, `zotero_search_by_tag`, `zotero_advanced_search`, `zotero_grep_text`, `zotero_search_notes`, `zotero_get_item`, `zotero_get_item_citation`, `zotero_get_fulltext`, `zotero_get_notes`, `zotero_get_annotations`, `zotero_get_attachments`, `zotero_get_collections`, `zotero_list_groups`, `zotero_get_collection_items`, `zotero_create_collection`, `zotero_find_or_create_collection`, `zotero_create_items`, `zotero_update_items`, `zotero_add_items_to_collection`, `zotero_create_attachment_link`. Auto-starts the local Zotero connector. Gated by `Feature::ResearchZotero`.
- **Merge plan**: Reapply `ResearchToolkit::is_tool_configured` write-vs-read split.

### 3. Hacker News Search & Threads
- **Type**: Local-only
- **Description**: Search HN via Algolia, fetch nested comment trees with depth/comment-count limits.
- **Implementation**: `codex-research-tools/src/clients/hackernews.rs`, `src/tools/hackernews.rs`. Tools: `hn_search`/`search_hackernews`, `hn_get_thread`/`get_hackernews_thread`. Skill assets: `skills/src/assets/research/hn-discoverer/`, `hn-synthesis/`, `hn-synthesizer/`. Gated by `Feature::ResearchHackerNews`.
- **Merge plan**: Reapply skill asset directories.

### 4. Patent Search (EPO Open Patent Services)
- **Type**: Local-only
- **Description**: Worldwide patent search via the European Patent Office API with OAuth.
- **Implementation**: `codex-research-tools/src/clients/{patents,epo_auth}.rs`, `src/tools/patents.rs`. Tools: `patent_search`/`search_patents`, `patent_get`/`get_patent`. `EpoAuthManager` for OAuth token refresh. Requires `EPO_CONSUMER_KEY`/`EPO_CONSUMER_SECRET`. Gated by `Feature::ResearchPatents`.
- **Merge plan**: Reapply with config fields.

### 5. Repo Analysis Suite (GitHub)
- **Type**: Local-only
- **Description**: Shallow-clone GitHub repos and statically extract: directory summary, model class definitions, dependency requirements, training/eval entrypoints, IO shape hints, repo health/maintenance signals, model export paths, training config schema, and diff against a local requirements file.
- **Implementation**: `codex-research-tools/src/clients/github.rs`, `src/tools/repo_analysis.rs` (~9 functions, on-disk repo cache w/ LRU eviction). Tools: `repo_clone_and_summarize`, `repo_find_models`, `repo_extract_requirements`, `repo_find_entrypoints`, `repo_extract_io_shapes`, `repo_get_health`, `repo_find_export_paths`, `repo_extract_config_schema`, `repo_diff_requirements`. Gated by `Feature::ResearchRepoAnalysis`.
- **Merge plan**: Self-contained.

### 6. Knowledge-Base & Synthesis Skills
- **Type**: Local-only
- **Description**: Skill-driven workflows that orchestrate the research tools (paper synthesis, cross-paper reports, conversation reports, research briefings, paper/HN discovery). Optional KB persistence is gated by `Feature::ResearchKnowledgeBase`.
- **Implementation**: `codex-rs/skills/src/assets/research/` containing 11 skill directories. Plus `core/src/research/researcher_prompt.rs` (`RESEARCHER_SYSTEM_PROMPT`) and `core/src/research/prompt.rs` (`build_research_prompt`).
- **Merge plan**: Reapply directory.

### 7. Living Reading View — Document Reader Tools
- **Type**: Local-only
- **Description**: Replaces inline-chat papers/synthesis output with an interactive document UI. Five model-facing tools let the agent declare and progressively fill a structured doc with sections.
- **Implementation**: `core/src/tools/handlers/document_reader.rs` (~1450 LOC). Tools registered in `core/src/tools/spec.rs:2207-2222` when `Feature::ReadingView` is on: `present_reading_view`, `update_document_section`, `append_to_section`, `add_document_section`, `patch_document_section`. Protocol types in `codex-protocol/src/document_reader.rs`. `ReadingViewDisplayMode` enum, follow-up guidance helpers, document cache & streaming-section reminder logic.
- **Merge plan**: Handler is self-contained but registered inside `spec.rs` — reapply the `if config.features.enabled(Feature::ReadingView)` block.

### 8. Reading View HTTP/WebSocket Server (Browser Mode)
- **Type**: Local-only
- **Description**: Local HTTP+WS server hosting the embedded `LivingReadingView.html`. Streams reading-view events to a browser tab; receives bidirectional messages.
- **Implementation**: Crate `codex-reading-view-server` (`reading-view-server/Cargo.toml`, `src/lib.rs`, embedded HTML). Built on `axum` + `tower-http` + `tokio::sync::broadcast`. Used in TUI via `tui/src/chatwidget_document_reader.rs::ensure_reading_view_server`.
- **Merge plan**: Reapply workspace entry + tui dep + integration.

### 9. Crop & Store PDF Figure Tool
- **Type**: Local-only
- **Description**: Render a PDF page (via `pdfium-render`) and crop a fractional bounding box, producing a PNG asset.
- **Implementation**: `core/src/tools/handlers/crop_figure.rs`. Tool spec `crop_and_store_figure` in `core/src/tools/spec/workspace.rs`. `pdfium_downloader` ensures the dynamic library is present in `~/.ata/lib/`.
- **Merge plan**: Reapply alongside reading-view registration.

### 10. Attach URL Files (PDF/Doc Pre-fetch)
- **Type**: Local-only
- **Description**: Fetch and cache up to N URLs per call, validating SSRF safety, attaching them to the conversation context.
- **Implementation**: `core/src/tools/handlers/attach_url_files.rs`, with `core/src/tools/url_downloader.rs` and `core/src/tools/url_validation.rs`.
- **Merge plan**: Reapply via `register_attach_url_files` call (`spec.rs:2430`).

### 11. Dataset Discovery & Download (HuggingFace + Kaggle)
- **Type**: Local-only
- **Description**: Search/get/list/download datasets across HuggingFace and Kaggle. Plus Kaggle competitions list/files/download.
- **Implementation**: Crate `codex-data-tools`. Clients: `src/clients/{huggingface,kaggle}.rs`. Tool layer `src/tools/dataset_ops.rs`. Cargo features: `huggingface`, `kaggle`. Tools: `dataset_search`/`search_datasets`, `dataset_get`/`get_dataset`, `dataset_list_files`/`list_dataset_files`, `dataset_download`/`download_dataset`, `hf_dataset_info`/`get_huggingface_dataset`, `kaggle_dataset_info`/`get_kaggle_dataset`, `kaggle_competitions`/`list_kaggle_competitions`, `kaggle_competition_list_files`, `kaggle_competition_download`. Wired into core via `core/src/tools/handlers/data.rs`, gated behind cargo feature `data` and `Feature::Data`.
- **Merge plan**: Add workspace member + cargo feature.

### 12. ElevenLabs Streaming TTS + STT Client
- **Type**: Local-only
- **Description**: Low-level client crate that powers ATA voice mode. Streaming TTS via WebSocket yielding 24 kHz mono PCM with character-level alignment for karaoke. STT via `POST /v1/speech-to-text`.
- **Implementation**: Crate `codex-elevenlabs` (`src/{tts,stt,types,error}.rs`). `TtsStream::connect` returns `mpsc<TtsChunk>`. Used by `tui/src/chatwidget/voice_mode.rs` and `tui/src/voice.rs`. Routes audio chunks into `AppEvent::VoiceModeTtsAudioChunk { alignment }`. Gated by `Feature::VoiceMode`.
- **Merge plan**: New crate; add workspace member + dep.

### 13. TreeSitter Standalone Code Intelligence
- **Type**: Local-only
- **Description**: Tree-sitter-based code-intel layer used by ATA's research-engineering subagents. Provides project indexing, symbol tables, callers/tests/variables lookup, structure peeking, multi-language grep, chunking with indices, and a dual-storage annotation system.
- **Implementation**: Crate `codex-treesitter`. Modules: `parser`, `project_index`, `symbol_table`, `symbol`, `file_entry`, `file_tree`, `walker`, `chunking`, `content`, `ops`, `annotations`, `queries/{rust,python,typescript,javascript,go,java,scala}.rs`. Wired into core via `core/src/tools/handlers/code_intel.rs` registering tool `code_intel` with 20 operations. Multi-root state in `core/src/state/`. Gated by `Feature::TreeSitter` and cargo feature `treesitter`.
- **Merge plan**: Workspace member + dep + reapply gates in `codex.rs` and `codex_tests.rs` (~10 sites).

### 14. Research Subagent Prompt System
- **Type**: Local-only
- **Description**: Multi-phase researcher prompt construction with availability-driven branches.
- **Implementation**: `core/src/research/prompt.rs` (`ResearchPromptParams`, `build_research_prompt`), `researcher_prompt.rs`, `tool_names.rs`, `output_schema.rs`, `types.rs`.
- **Merge plan**: Pure additions in `core/src/research/`.

### 15. ResearchToolkit / DataToolkit Lifecycle (Per-Thread)
- **Type**: Local-only
- **Description**: Lazily-instantiated, shared across a thread. `OnceCell<Arc<SharedResearchToolkit>>` and `OnceCell<Arc<SharedDataToolkit>>` constructed in `ThreadManager`.
- **Implementation**: `core/src/thread_manager.rs:164-166, :711, :733`. `core/src/codex.rs:393-394` (`research_toolkit`/`data_toolkit` fields on `TurnContext`). Router signatures threaded through.
- **Merge plan**: These additions touch upstream-shared structs — expect conflicts on every merge.

### 16. Tool-Name Resolution / Aliasing (MCP ↔ Native)
- **Type**: Local-only
- **Description**: Maps MCP-style tool names (`search_papers`) to native names (`paper_search`).
- **Implementation**: `core/src/research/tool_names.rs`, `core/src/data/tool_names.rs`.
- **Merge plan**: Self-contained.

### 17. Voice-Mode Reading View Integration (TTS Narration of Sections)
- **Type**: Local-only
- **Description**: When voice mode is active, TUI auto-narrates current reading-view section via TTS, with karaoke highlighting, pre-generation of adjacent-section audio, pause/resume/interrupt, and playback-speed control.
- **Implementation**: `tui/src/app_event.rs` arms `VoiceModeNarrateSection`, `VoiceModeTtsAudioChunk { alignment }`, `VoiceModeTtsFinished`, `VoiceModeTtsError`, `VoiceModeHighlightTick`, `VoiceModeInterruptTts`, `VoiceModePauseTts`, `VoiceModeResumeTts`, `VoiceModePlaybackSpeedChange`. Wired in `tui/src/chatwidget/voice_mode.rs`.
- **Merge plan**: Significant `tui` divergence in `app_event.rs` and `chatwidget/`.

### 18. Research/Voice/Reading Feature Flags
- **Type**: Local-only
- **Description**: New entries in the `Feature` enum: `Research`, `ResearchPaperSearch`, `ResearchZotero`, `ResearchHackerNews`, `ResearchPatents`, `ResearchRepoAnalysis`, `ResearchKnowledgeBase`, `Data`, `ReadingView`, `VoiceMode`, `TreeSitter`, `Lsp`, `Coordination`.
- **Implementation**: `core/src/features.rs` lines 162, 196-218, 461-492, 557-565, 832-1014. Includes `is_tool_id_enabled` dispatch logic.
- **Merge plan**: Heavy conflicts; reapply enum variants and `FEATURES` array entries.

### 19. Zotero Skill (Agent Workflow)
- **Type**: Local-only
- **Implementation**: `skills/src/assets/research/zotero/`.
- **Merge plan**: Reapply.

### 20. Workspace + Build Wiring
- **Type**: Local-only
- **Implementation**: `codex-rs/Cargo.toml` members lines `71-79` and workspace path deps lines `114-136`. `core/Cargo.toml:48-51,134-136` (data/lsp/treesitter cargo features). `tui/Cargo.toml:46,118,142`.
- **Merge plan**: Always reapply these workspace entries first.

### Summary

Upstream **does** have:
- A `web_search` tool concept — but only as a model-provider hosted tool, not academic-content-aware.
- An `agent_jobs` handler — unrelated to research scheduling.
- An `lsp_workspace_edit`/`lsp` handler — partial overlap with `code_intel`, but tree-sitter is local-only.

There is **no upstream equivalent** for any of the 20 features above.
