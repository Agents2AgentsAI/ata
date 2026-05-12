## Wave-2 Feature Flag Registry Analysis (Agent 14)

### Source maps
- Local registry: `codex-rs/core/src/features.rs` (1101 lines, 67 `Feature` variants, 65 `FEATURES` specs).
- Upstream registry: `codex-rs/features/src/lib.rs` at tag `rust-v0.129.0` (separate `codex-features` workspace crate, 75 variants, depends on `codex-otel` + `codex-protocol` only).
- Local has no `codex-rs/features/` crate. Upstream has fully extracted the registry into a standalone crate.

### Structural deltas (registry-level, not per-flag)
1. **Crate placement.** Upstream pulls features out of `core` into its own crate `codex-features`. Local still keeps it inside `codex_core::features` and imports `Config`, `ConfigToml`, `ConfigProfile`, `AuthManager`, `CodexAuth`, plus `codex_protocol::*` directly — tight coupling that upstream has explicitly broken.
2. **`from_config` vs `from_sources`.** Local exposes `Features::from_config(&ConfigToml, &ConfigProfile, FeatureOverrides)`. Upstream replaced this with a config-agnostic `Features::from_sources(base: FeatureConfigSource, profile: FeatureConfigSource, overrides)`. ~10+ `from_config` callers in local will need to be rewritten.
3. **`apps_enabled` lives in `core` upstream.** Upstream's `Features` only exposes `apps_enabled_for_auth(has_chatgpt_auth: bool)`. Local has both async `apps_enabled(&AuthManager)` and `apps_enabled_cached(&AuthManager)` directly on `Features`. Reconcile by moving the auth-resolving wrappers into `core::auth`.
4. **`FeaturesToml`.** Local is a flat `BTreeMap<String, bool>`. Upstream adds typed structured sub-features (`multi_agent_v2`, `apps_mcp_path_override`) using a `FeatureToml<T>` enum (`Enabled(bool) | Config(T)`) plus a `FeatureConfig` trait — and adds `materialize_resolved_enabled(&Features)` so the resolved set can be round-tripped to TOML.
5. **`maybe_push_unstable_features_warning` shape.** Local takes `&Config` and pushes into a `Vec<Event>`; skips warnings for `is_research_feature(...)`. Upstream's `unstable_features_warning_event` returns `Option<Event>`, takes `effective_features: Option<&Table>` + `&Features` + `&str` config path, no research carve-out.
6. **Stage variants.** Match. Both define `UnderDevelopment | Experimental{...} | Stable | Deprecated | Removed`.

### Per-feature divergence (selected; 22 items)

#### 1. ResearchPaperSearch — **Local-only**
- Variant: `ResearchPaperSearch` (key `research_paper_search`, `Stage::Stable`, default **on**).
- Gates: research paper-search tools and the `paper-*` skill family.
- Sites: `core/src/research/tool_names.rs:321`, `core/src/features.rs:465,484`, `tui/src/bottom_pane/research_tools_view.rs:38`.
- Merge plan: **Keep**.

#### 2. ResearchZotero / ResearchHackerNews / ResearchPatents / ResearchRepoAnalysis — **Local-only**
- Keys `research_zotero`, `research_hacker_news`, `research_patents`, `research_repo_analysis`. HN is `Stable` default-on; the other three are `UnderDevelopment` default-off.
- Sites: `core/src/research/tool_names.rs:162,326,331,336`, `core/src/features.rs:466-469`, `tui/src/bottom_pane/research_tools_view.rs:44-62`.
- Merge plan: **Keep all four**.

#### 3. ReadingView — **Local-only**
- Key `reading_view`, `Stage::Stable`, default **on**.
- Sites: `core/src/codex.rs:3781`, `core/src/tools/spec.rs:2207`, `tui/src/chatwidget_document_reader.rs:411`, `tui/src/bottom_pane/research_tools_view.rs:68/293/388`.
- Merge plan: **Keep**. Comment notes "ReadingView requires Research" — keep `normalize_dependencies` to enforce that.

#### 4. ResearchKnowledgeBase — **Local-only**
- Key `research_knowledge_base`, `Stage::Stable`, default on. Gates the `kb` skill and `conversation-report` skill.
- Merge plan: **Keep**.

#### 5. Research (master toggle) — **Local-only**
- Key `research`, `Stage::UnderDevelopment`, default off. When on, all research tool-ids and skill names short-circuit to true.
- Sites: 11 usages.
- Merge plan: **Keep**.

#### 6. VoiceMode — **Local-only**
- Key `voice_mode`, `Stage::Experimental`, default off.
- Merge plan: **Keep**.

#### 7. VoiceTranscription — **Local-only**
- Key `voice_transcription`, `Stage::UnderDevelopment`, default off.
- Merge plan: **Keep**.

#### 8. Lsp — **Local-only**
- Key `lsp`, `Stage::Stable`, default **on**.
- Sites: `core/src/codex/code_intel.rs:230`, `core/src/tools/spec.rs:2704`.
- Merge plan: **Keep**.

#### 9. TreeSitter — **Local-only**
- Key `treesitter`, `Stage::Stable`, default **on**.
- Sites: `core/src/codex/code_intel.rs:236`, `core/src/tools/spec.rs:2720`.
- Merge plan: **Keep**.

#### 10. Coordination — **Local-only (private!)**
- Key `coordination`, `Stage::UnderDevelopment`, default off.
- Sites: `core/src/codex.rs:1859`, `core/src/tools/spec.rs:2582`.
- Merge plan: **Keep, private-only**. Per `codex-rs/CLAUDE.md`, the `coordination/` crate must never reach the `release` branch.

#### 11. Data — **Local-only**
- Key `data`, `Stage::Experimental` ("Data Tools"), default off.
- Merge plan: **Keep**.

#### 12. Artifact — **Shared (likely identical)**
- Local: key `artifact`, `Stage::UnderDevelopment`, default off.
- Upstream: key `artifact`, `Stage::UnderDevelopment`, default off — identical.
- Merge plan: **Adopt upstream verbatim**.

#### 13. Scheduler — **Local-only**
- Key `scheduler`, `Stage::Experimental`, default off. Background job scheduler.
- Merge plan: **Keep**.

#### 14. AppsMcpGateway — **Local-only**
- Declared at `core/src/features.rs:158`. "Route apps MCP calls through the configured gateway."
- Merge plan: **Keep** but verify whether it overlaps with upstream's new `AppsMcpPathOverride`.

#### 15. PowershellUtf8 — **Local-only**
- Key `powershell_utf8`. `cfg(windows)` → `Stage::Stable`/default true; otherwise `UnderDevelopment`/false.
- Merge plan: **Keep**.

#### 16. JsRepl / JsReplToolsOnly — **Drift**
- Local: live (`JsReplToolsOnly` requires `JsRepl` enforced in `normalize_dependencies` line 504-507).
- Upstream: both `Stage::Removed`.
- Merge plan: **Keep local live**. ATA still ships the JS REPL feature.

#### 17. ImageDetailOriginal — **Drift**
- Local: `Stage::UnderDevelopment`, default off, **live** (gates behavior at `core/src/original_image_detail.rs:10`).
- Upstream: `Stage::Removed`.
- Merge plan: **Keep local live**.

#### 18. CodexHooks — **Drift in key + stage**
- Local: key **`codex_hooks`**, `Stage::UnderDevelopment`, default off.
- Upstream: key **`hooks`**, `Stage::Stable`, default **on**.
- Merge plan: **Adopt upstream key+stage**. Add legacy alias `codex_hooks → hooks` in `legacy.rs`.

#### 19. ShellSnapshot — **Drift in stage**
- Local: `Stage::Experimental`, default off.
- Upstream: `Stage::Stable`, default **on**.
- Merge plan: **Adopt upstream**.

#### 20. Apps — **Drift in stage**
- Local: `Stage::Experimental`, default off.
- Upstream: `Stage::Stable`, default **on**.
- Sites: 30 references.
- Merge plan: **Likely keep local Experimental + default-off** because ATA's Apps pipeline still hangs on ChatGPT auth.

#### 21. GuardianApproval — **Drift in stage**
- Local: `Stage::Experimental` (long ATA-customized menu copy), default off.
- Upstream: `Stage::Stable`, default **on**.
- Sites: 43 references.
- Merge plan: **Adopt upstream Stable+default-on** but **keep ATA's menu copy**.

#### 22. Personality / FastMode / SkillMcpDependencyInstall / EnableRequestCompression / Collab / ShellTool / UnifiedExec / UseLegacyLandlock — **Shared, identical**
- All 8 match upstream. `UseLegacyLandlock` is `Stable` default-off in local, `Deprecated` default-off in upstream — minor stage drift; adopt upstream `Deprecated`.

#### 23. Upstream-new variants ATA does not have (12+)
- `TerminalResizeReflow` (Experimental, default-on)
- `ApplyPatchStreamingEvents` (UnderDevelopment)
- `BuiltInMcp` (UnderDevelopment)
- `Chronicle` (UnderDevelopment)
- `MultiAgentV2` (UnderDevelopment, structured config `MultiAgentV2ConfigToml`)
- `EnableMcpApps`, `AppsMcpPathOverride` (structured config), `ToolSearch` (Stable default-on), `ToolSearchAlwaysDeferMcpTools`, `UnavailableDummyTools` (Stable default-on), `PluginHooks`, `RemotePlugin`, `InAppBrowser`, `BrowserUse`, `BrowserUseExternal`, `ComputerUse` (last 4: Stable default-on)
- `ExternalMigration`, `AuthElicitation`, `Goals`, `RemoteControl`, `WorkspaceOwnerUsageNudge`, `ResponsesWebsocketResponseProcessed`, `RemoteCompactionV2`, `WorkspaceDependencies` (Stable default-on)
- Merge plan: **Adopt all that are not gated by upstream-only product features**. `ToolSearch` + `UnavailableDummyTools` + `ToolSearchAlwaysDeferMcpTools` should be adopted with their gating logic as they materially change tool exposure.

#### 24. `normalize_dependencies` divergence
- Local has extra rule: `JsReplToolsOnly` requires `JsRepl`, otherwise warn-and-disable.
- Merge plan: **Keep local rule**.

#### 25. `is_research_feature` carve-out
- Local skips warnings for any of `Research | ResearchPaperSearch | ResearchZotero | ResearchHackerNews | ResearchPatents | ResearchRepoAnalysis | ReadingView | ResearchKnowledgeBase`.
- Upstream warns on every `UnderDevelopment` feature.
- Merge plan: **Keep local carve-out**.

### Summary — reconciliation strategy

The cleanest landing is to **adopt upstream's `codex-features` crate as the new home for the registry**, then layer ATA's deltas as additive variants, additive `FEATURES` rows, and an additive `is_research_feature` carve-out. Concretely:
1. Introduce `codex-rs/features/` as a new workspace member containing upstream's lib.
2. Re-export `pub use codex_features::*` from `codex_core::features`.
3. Keep ATA-only auth wrappers (`apps_enabled`, `apps_enabled_cached`) in `core::auth`.
4. Append ATA-only variants (`Research*`, `ReadingView`, `VoiceMode`, `VoiceTranscription`, `Lsp`, `TreeSitter`, `Coordination`, `Data`, `Scheduler`, `AppsMcpGateway`, `PowershellUtf8`, `ResearchKnowledgeBase`).
5. Selectively reject upstream's `Removed` markers for `JsRepl`, `JsReplToolsOnly`, `ImageDetailOriginal`.
6. Adopt upstream's promotions for `ShellSnapshot`/`GuardianApproval`/`CodexHooks` (rename `codex_hooks` → `hooks` with legacy alias).
7. Keep ATA's `is_research_feature` carve-out.

This collapses the registry diff to ~15 ATA-only rows plus a handful of stage tweaks.
