## Wave-2 Commit Archeology (Agent 12)

Total fork-only commits: **592** (since merge base `926b2f19e8c`, span 2026-02-04 → 2026-03-25).

### Cluster 1 — Multi-Provider Foundation (Anthropic / Gemini / OpenAI)
- **Volume:** ~25 commits, 2026-02-04 → 2026-02-13.
- **Type:** Local-only, structural — entirely new layer on top of upstream's OpenAI-only client.
- **Reps:** `3ef4c46239 codex rs: add other providers`, `9403518e3c providers: add models`, `c09292a58b oauth: initial gemini`.
- **Plan:** Preserve. Foundational. Re-apply the provider abstraction; rebase any upstream changes to the OpenAI client *into* the abstraction.

### Cluster 2 — Research Agent / Research Tools Crate
- **Volume:** ~22 commits, 2026-02-07 → 2026-03-05.
- **Type:** Local-only crate (`codex-research-tools`).
- **Reps:** `eb42737b9e research tools: crate skeleton`, `effc73893f research: research scout, pdf figures, latex`.
- **Plan:** Preserve. Cargo features `research-all`, `research-zotero`, etc.

### Cluster 3 — Zotero Integration (CLI tools, not MCP)
- **Volume:** ~24 commits, 2026-02-11 → 2026-02-25.
- **Type:** Local-only — explicitly migrated *off* MCP into native ATA tools.
- **Reps:** `c774276ee2 zotero: remove mcp specs`, `5d734d8803 zotero: remove mcp`, `4564af7d76 zotero: allow local instance`.
- **Plan:** Preserve. Don't let upstream MCP examples reintroduce a Zotero MCP.

### Cluster 4 — PDF Pipeline (native attachments, URLs, figure extraction)
- **Volume:** ~28 commits, 2026-02-04 → 2026-03-05.
- **Type:** Local-only feature crate / shared with provider clients.
- **Reps:** `5f8b2aab5b pdf: add utils to handle files`, `5a1d174d29 pdf: handle urls`, `d31e10d58e compaction: fix compaction with pdfs`.
- **Plan:** Preserve. Cross-cutting; re-apply early.

### Cluster 5 — Reading View / Sectioned Reading Mode
- **Volume:** ~14 commits, 2026-02-14 → 2026-03-20.
- **Reps:** `b993b7f2cf feat: sectioned reading mode with append/patch tools`, `944bb954a8 reading-view: feature flag`, `9018f1a4b7 reading view: scroll with mouse`.
- **Plan:** Preserve. Has its own feature flag.

### Cluster 6 — Voice Mode (TUI dictation + setup wizard)
- **Volume:** ~16 commits, 2026-02-28 → 2026-03-10.
- **Type:** Shared-but-extended — upstream added basic voice; ATA layered a setup view, API-key prompt, language/speed picker, pause/resume, bracketed-paste, karaoke prefix.
- **Reps:** `5d9868850b voice mode`, `37e2c97191 voice: align setup view brackets`, `1a0afdb536 voice: fix pause, fix crash, fix resume`.
- **Plan:** Adopt-upstream-base + re-apply ATA layer.

### Cluster 7 — TTS / Karaoke
- **Volume:** ~8 commits, 2026-02-28 → 2026-03-10.
- **Reps:** `f2ee5bc35e karaoke animation`, `bd6e2bdfa2 voice: use ◆ for karaoke prefix`.
- **Plan:** Preserve. Distinct subsystem (worker + history-pane animation).

### Cluster 8 — LSP / Code Intel (`codex-lsp`)
- **Volume:** ~36 commits, 2026-03-01 → 2026-03-05 (largest single-feature spike).
- **Reps:** `cc544f8e4f lsp: initial commit`, `c3cb5c9c99 build: enable lsp and treesitter by default`, `241fd87b9d core.rs: factor out code intel`.
- **Plan:** Preserve. Search for both `lsp` and `code intel` names during conflict resolution.

### Cluster 9 — Tree-sitter Multi-Root
- **Volume:** ~7 commits, 2026-03-02.
- **Reps:** `075c1bdda5 treesitter: initial codes`, `45c35a74e1 treesitter: add multi-root`.
- **Plan:** Preserve.

### Cluster 10 — Workspaces Skill / Workspace Repo Spec
- **Volume:** ~28 commits, 2026-03-01 → 2026-03-10.
- **Type:** Local-only — moved from markdown skill to Rust implementation mid-cluster.
- **Reps:** `1622937b96 workspace: add skill`, `72ba858fb2 workspace: move skill implementation to rust`, `92c63d5eeb workspaces: implement workspace repo spec`.
- **Plan:** Preserve. The Rust port supersedes the markdown skill.

### Cluster 11 — Scheduler Daemon + `/jobs` + `job-manager` Skill
- **Volume:** ~9 commits, 2026-02-27 → 2026-02-28.
- **Reps:** `43bcbc7a0a feat: add codex-scheduler crate`, `875409d97d feat: add job-manager skill and /jobs TUI command`, `2cb7d19e9b fix: add scheduler dirs to sandbox writable roots`.
- **Plan:** Preserve. Sandbox-writable-roots tweak is non-obvious.

### Cluster 12 — Mobile / Remote Control / Coordination Relay
- **Volume:** ~10 commits, 2026-02-27 → 2026-03-10.
- **Reps:** `2eca7865da remote-control`, `0d18c4643c feat: add agent coordination channel for cross-session awareness`, `7b38055adb agent coordination relay`.
- **Plan:** Preserve.

### Cluster 13 — Auth / OAuth Refactor (Gemini OAuth, multi-provider keyring)
- **Volume:** ~13 commits, 2026-02-08 → 2026-03-20.
- **Reps:** `8586617de7 auth: extraction`, `5d5d99c992 provider auth: refactor`, `c09292a58b oauth: initial gemini`, `0b482a90bf auth: fix multiple signin for keyring`.
- **Plan:** Preserve, rebase on upstream.

### Cluster 14 — KB / Paper-Synthesis / Skills (research-flag-gated)
- **Volume:** ~10 commits, 2026-02-12 → 2026-03-10.
- **Reps:** `394760658d kb`, `90d8b10fa8 rm kb tools, add kb skills, improve document reader`, `3c893c5be4 kb and paper synthesis skkills to depend on research flag`.
- **Plan:** Preserve. Don't bring back the tool form.

### Cluster 15 — Telemetry Disable / OpenAI Endpoint Removal
- **Volume:** ~5 commits, 2026-02-15 → 2026-03-05.
- **Reps:** `a4070902c3 telemetry: disable by default and remove OpenAI endpoint`, `b14e9619d0 telemetry: complete`.
- **Plan:** Preserve. Critical privacy posture. Audit upstream for any new telemetry hooks during merge.

### Cluster 16 — Branding / Rebrand Codex → ATA
- **Volume:** ~25 commits, 2026-02-15 → 2026-02-21.
- **Reps:** `c1fd2ffebe brands: codex to ata`, `b201ae47d2 npm: change names for release`, `0007ad2c41 brew cask`.
- **Plan:** Preserve. Mechanical re-application.

### Cluster 17 — Models / Reasoning Effort / Model Picker
- **Volume:** ~12 commits, 2026-02-04 → 2026-02-27.
- **Type:** Shared-but-extended.
- **Reps:** `9403518e3c providers: add models`, `582c81674b models: add reasoning efforts`, `beb530c2c5 models: use local presets for metadata`.
- **Plan:** Adopt-upstream + re-apply ATA presets.

### Cluster 18 — Release Pipeline / Dependabot / CI
- **Volume:** ~10 commits, 2026-02-04 → 2026-03-10.
- **Reps:** `5c92e3a41b dependabot: disable`, `eb96170415 rust ci: add bubblewrap back`, `07756c4373 ci: re-apply unprivileged user namespaces fix`.
- **Plan:** Preserve. Some commits are upstream-undo and need re-checking after merge.

### Cluster 19 — Compaction Adjustments
- **Volume:** ~3 commits.
- **Reps:** `d1d854e6d5 compaction: fix handling of pdf files`, `8d77e6f5a8 compaction: switch to upstream`, `d31e10d58e compaction: fix compaction with pdfs`.
- **Plan:** Take upstream's compaction; re-apply only the PDF-aware patches.

### Cluster 20 — Big "kitchen sink" feat commit on top
- **Volume:** 1 commit, 2026-03-20.
- **Type:** Re-roll commit — `29ff511925 feat: reading view, TTS, voice mode, auth, and figure extraction`.

### Feature-Flag / Stage Commits (gating callouts)

- `c3cb5c9c99 build: enable lsp and treesitter by default`
- `944bb954a8 reading-view: feature flag`
- `fa4ff96825 research-all feature, and make the kb in core`
- `abf2b8276f research tools: remove feature gates`
- `40e58dc2b3 research features: more fixes for switching feature`
- `3953a211cc research: fix zotero feature switching`
- `4636b3b44c feature flag: revert changes` — partial revert; check what got rolled back
- `2ac6385932 fix: CI lint errors - cfg gates, schema regen`
- `7aa3f15846 fix: gate voice-only code with cfg for Linux compilation`
- `bd420e9c28 fix: gate all voice-only methods/types with cfg for Linux dead_code`

Across the board there are at least four feature axes: `research-all` / `research-zotero` / `reading-view` / `lsp` + `treesitter` / per-platform `cfg(target_os = "linux")` voice gates.

### Possibly Missed by Wave-1

1. **Mobile CLI subcommand + QR/remote-control pairing.** This is a *coordination relay client*, not a standalone tool.
2. **Agent Coordination Channel / Cross-Session Awareness.** Server-side cross-session pubsub.
3. **`codex-scheduler` daemon + `--daemon` flag + sandbox-writable-roots widening.** The sandbox change is easy to lose during merge.
4. **`/jobs` TUI slash command + `job-manager` skill (with Playwright/external-service hooks).** Skill ships an external-service setup playbook.
5. **Workspaces "Repo Spec" + workspace root resolution.** Distinct from upstream "workspaces" terminology — multi-repo.
6. **Tree-sitter multi-root state.** Distinct from LSP — separate gating.
7. **Sectioned Reading Mode with append/patch tools.** Adds new tool surface, not just a viewer.
8. **Karaoke animation prefix glyphs (`◆`, `♪`, bullet) + `/voice-setup`.**
9. **Hackernews source + figure extraction.**
10. **`research scout` + LaTeX figures pipeline.** Note: `c913698c0b tools: remove latex` later partially walks back LaTeX.
11. **`zotero: allow local instance` + groups/annotations/grep tools.** Zotero surface is much bigger than "search" — at least 7 tools.
12. **Telemetry disable + OpenAI endpoint removal as a privacy posture.**
13. **Reverse-search + tutorial + keymap overhaul tied to reading view.** `e6ecb5144d`.
14. **`auth: defer mcp`** — non-obvious behavioral change around when MCP auth fires.
15. **`research_command` PR → `/research` slash command.** `f33ce35e0a`, `01e550f8e1 research: add slash command back`. The "back" suggests it was removed and re-added.
16. **Mouse scroll fixes for the chat history.** `6d006fe195`.
17. **`tui: frames for log`** (`dc0a2927aa`).

No evidence of: presentation/`pptx` tool, supabase auth, ghost-commit/undo, package-manager skill, or memories-clear command on the fork side — those appear only inside upstream NEW-FEATURES blob commits which were imported, not authored locally. So if wave-1 catalogued them as ATA features, they should be re-classified as "upstream-imported, possibly customized".
