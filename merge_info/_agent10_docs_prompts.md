## Docs/Prompts Analysis (Agent 10)

Upstream tag: `rust-v0.129.0`. Local branch: `merge_upstream_0.129.0`. Diff scope: 336 paths under `*.md`, `*.toml`, `codex-rs/skills/**`, `codex-rs/core/templates/**`, `codex-rs/core/src/prompts/**`, and `docs/**`.

### 1. Root `AGENTS.md` — Shared (heavily diverged)
- **Type:** Shared (both have the file; substantial fork-only additions)
- **Description:** Top-level repo agent guidance for contributors and the embedded coding agent.
- **Implementation:** `AGENTS.md`. Read at repo root by AGENTS.md discovery; the persona prompts (e.g. `codex-rs/core/templates/agents/orchestrator.md`) reference and rely on AGENTS.md scope.
- **Local additions:** brand-new "User interaction principles → Minimize user friction for external services" preamble (rules 1–6 + bad/good examples), a "Release branch & public/private separation" section pointing at `codex-rs/CLAUDE.md`, and reworded test/lint commands (`just test-research`, `just fix-fast`). Drops upstream-only rules (RPITIT trait shape, `chatwidget.rs` size discipline, `codex-mcp` mutation rule, "resist adding code to codex-core").
- **Merge plan:** **Merge.** Adopt upstream's restored rules where they don't conflict with the fork (RPITIT trait shape, codex-core warning), keep the fork-only "user friction" preamble and "Release branch" section verbatim. Re-write the test-suite paragraph to keep `just test-research`/`just fix-fast` (fork tooling) instead of upstream's plain `just test`.

### 2. `codex-rs/AGENTS.md` — Local-only
- **Type:** Local-only (file is new in fork; absent at `rust-v0.129.0`).
- **Description:** Build & test iteration strategy for this 58-crate workspace — `cargo check` first, narrow `-p`/`--test`/`-- name` runs, nextest, four crate dependency tiers, slow-crate list, macOS lld linker, incremental cache bloat fix, XProtect bypass.
- **Implementation:** `codex-rs/AGENTS.md` (111 lines). Read in-tree by AGENTS.md scope rules whenever the agent works in `codex-rs/`.
- **Merge plan:** **Keep verbatim.** Pure performance/dev-loop guidance — does not contradict any upstream content.

### 3. `codex-rs/CLAUDE.md` — Local-only
- **Type:** Local-only.
- **Description:** Public/private separation playbook: lists private-only paths (`coordination/`, `coordination-relay/`, `supabase/`, `skills/src/assets/remote-exec/`, `core/src/coordination_context.rs`, `core/src/tools/handlers/team_post.rs`, `exec/src/lib.rs` `relay` cfg), describes `just sync-release`, `_release_mixed_files`, and the Prompt Inspector workflow (`just prompts`, `// @agent-facing`, `tools/prompt-inspector/prompt-registry.toml`, `just check-prompts`).
- **Implementation:** `codex-rs/CLAUDE.md` (46 lines). Cross-referenced from root `AGENTS.md` "Release branch" section.
- **Merge plan:** **Keep.** No upstream equivalent. This is the source of truth for the fork-vs-public boundary.

### 4. `codex-rs/ata-research-explainer.md` — Local-only persona blurb
- **Type:** Local-only.
- **Description:** 5-line marketing/persona description of "Ata" (research engineering system: papers, citations, hypotheses, experiments, multi-agent coordination, structured documents).
- **Implementation:** `codex-rs/ata-research-explainer.md`. **Note:** `grep` shows no `include_str!` consumer in the Rust tree — it appears to be a stranded/orphaned doc artifact, not wired into any prompt.
- **Merge plan:** **Investigate before keeping.** Either (a) wire it into the agent persona via `include_str!` if it was meant to be loaded, or (b) move to `docs/` and delete from `codex-rs/` to clean up the crate root. Do not silently retain in current location.

### 5. Core system prompts: brand rename Codex → Ata — Shared
- **Type:** Shared (every prompt file was edited).
- **Description:** First-line/header rebrand of the agent identity in all five system prompts.
- **Implementation:**
  - `codex-rs/core/prompt.md` — "You are Codex, based on GPT-5… Codex CLI" → "You are Ata, based on GPT-5… Ata CLI".
  - `gpt_5_codex_prompt.md`, `gpt_5_1_prompt.md`, `gpt_5_2_prompt.md`, `gpt-5.1-codex-max_prompt.md`, `gpt-5.2-codex_prompt.md` — same rebrand on line 1.
  - `prompt_with_apply_patch_instructions.md` — same rebrand + drops the "Within this context, Codex refers to…" disambiguation paragraph.
  - `protocol/src/prompts/base_instructions/default.md` — same rebrand + disambiguation paragraph removed.
- **Merge plan:** **Keep rebrand.** Re-apply the `Codex → Ata` and `Codex CLI → Ata CLI` renames after every upstream merge. Reapply the deletion of the "Within this context, Codex refers to…" sentence in `prompt_with_apply_patch_instructions.md` and `base_instructions/default.md`. Use the established sed-style script the team already runs.

### 6. `codex-rs/core/templates/agents/orchestrator.md` — Shared (large fork divergence)
- **Type:** Shared, but the local file is +64 lines of fork-only persona content.
- **Description:** Orchestrator/sub-agent system prompt. Local version adds tone/style rules, sub-agent flow, GIT/AGENTS.md handling, planning, and code-style guidance (precedence rules, file-path linking conventions, "never tell the user to save/copy this file", etc.).
- **Implementation:** `codex-rs/core/templates/agents/orchestrator.md`, embedded into the orchestrator agent builtin via the templates loader.
- **Merge plan:** **Merge.** Diff upstream changes into the local file but preserve every fork-only block (tone/style, sub-agent flow, AGENTS.md, GIT, file-path linking).

### 7. `codex-rs/core/templates/collaboration_mode/{default,execute,pair_programming,plan}.md` — Local-only collab system
- **Type:** Local-only (replaces upstream `templates/collab/experimental_prompt.md` + upstream `templates/personalities/*` + upstream `templates/compact/*`, all of which are gone in the fork).
- **Description:** Four-mode collaboration system (Default / Execute / Plan / Pair Programming) using `<collaboration_mode>` tagging, `{{KNOWN_MODE_NAMES}}`, `{{REQUEST_USER_INPUT_AVAILABILITY}}`, `{{ASKING_QUESTIONS_GUIDANCE}}` placeholders.
- **Implementation:** `codex-rs/core/templates/collaboration_mode/*.md` plus separate `codex-rs/collaboration-mode-templates/` crate (`templates/default.md`).
- **Merge plan:** **Keep.** This is a fork-specific UX/feature replacing upstream's experimental "collab" prompt. If upstream re-introduces a `collab/` prompt, evaluate whether it should be wired in alongside or replaced by the fork's mode system.

### 8. `codex-rs/core/templates/research/{researcher_system_prompt,zotero_developer_instructions}.md` — Local-only research personas
- **Type:** Local-only.
- **Description:**
  - `researcher_system_prompt.md` — 55-line "Research Persona" prompt (operating principles, method, output expectations, citation contract, sub-agent coordination).
  - `zotero_developer_instructions.md` — 25-line developer prompt for the `ata zotero ...` CLI namespace (status/collections/find-repos/resolve-paper guidance).
- **Implementation:** `researcher_system_prompt.md` is loaded via `include_str!` in `codex-rs/core/src/research/researcher_prompt.rs`. `zotero_developer_instructions.md` consumed by Zotero developer-instructions injection.
- **Merge plan:** **Keep.** Pure fork research feature; no upstream equivalent.

### 9. `codex-rs/core/templates/tools/presentation_artifact.md` — Local-only
- **Type:** Local-only (200 lines).
- **Description:** Tool description for the PowerPoint presentation artifact built-in tool — long action menu (`create`, `import_pptx`, `export_pptx`, layout/placeholder ops, undo/redo, set_theme, etc.).
- **Implementation:** `codex-rs/core/templates/tools/presentation_artifact.md`. Likely included via `include_str!` in the artifacts crate or templates-loaded.
- **Merge plan:** **Keep.** Fork-only feature.

### 10. `codex-rs/core/templates/search_tool/tool_suggest_description.md` — Local-only
- **Type:** Local-only (replaces upstream `request_plugin_install_description.md` which is removed in fork).
- **Description:** Description for the `tool_suggest` discovery flow (when no installed connector matches, suggest a discoverable connector/plugin with `tool_type` + `action_type` payloads).
- **Implementation:** `codex-rs/core/templates/search_tool/tool_suggest_description.md`. Companion to upstream-shared `tool_description.md`.
- **Merge plan:** **Keep.** Confirm upstream's newer `request_plugin_install_description.md` content has nothing the fork still wants; otherwise ensure both flows coexist.

### 11. `codex-rs/core/src/tools/code_mode/description.md` + `wait_description.md` — Local-only
- **Type:** Local-only (17 + 8 lines).
- **Description:** Tool descriptions for the JavaScript `exec` (code-mode) tool — sandboxed JS context, `// @exec:` pragma, `tools.*` global, helpers (`text`, `image`, `store`, `load`, `yield_control`, `ALL_TOOLS`).
- **Implementation:** `codex-rs/core/src/tools/code_mode/description.md`, `wait_description.md`. Embedded via `include_str!` in code-mode tool plumbing.
- **Merge plan:** **Keep.** Fork JS-REPL feature (see `docs/js_repl.md`).

### 12. Skills assets — Shared bucket, drastically reorganized
- **Type:** Mixed; the **fork removed all upstream sample skills (`skill-creator`, `skill-installer` major variants, `imagegen`, `plugin-creator` etc. as non-research) and added 14 fork-specific skills**.
- **Description:**
  - **Local-only research skills under `codex-rs/skills/src/assets/research/`:** `paper-discoverer`, `paper-discovery`, `paper-synthesizer`, `paper-synthesis`, `cross-paper-report`, `conversation-report`, `hn-discoverer`, `hn-synthesis`, `hn-synthesizer`, `kb`, `research-briefing`, `zotero` (all `SKILL.md` plus `agents/openai.yaml` for some).
  - **Local-only `adapt-environment/SKILL.md`** — fixes `ImportError`/version conflicts on GPU nodes.
  - **Local-only `workspace/SKILL.md` + 8 reference docs** — multi-repo workspace mgmt under `~/.ata/workspaces/`.
  - **Local-only `samples/job-manager/SKILL.md` (+189 lines)** — fork-specific job manager skill.
  - **Modified shared samples:** `samples/openai-docs/` swaps `prompting-guide.md`+`upgrade-guide.md`+`resolve-latest-model-info.js` for `gpt-5p4-prompting-guide.md`+`upgrading-to-gpt-5p4.md` (+620/-616). `samples/slides/` and `samples/spreadsheets/` add new `SKILL.md` and reference docs (`auto-layout.md`, `presentation.md`, `ranges.md`, `workbook.md`).
- **Implementation:** All embedded via `codex-rs/skills/build.rs` + `include_dir!` in `codex-rs/skills/src/lib.rs` (which itself diverged by +285 lines).
- **Merge plan:** **Per-skill triage.** Keep all `research/*`, `adapt-environment/`, `workspace/`, `samples/job-manager/`. For skills that exist in both (`samples/openai-docs/`, `samples/slides/`, `samples/spreadsheets/`), diff each upstream change and merge content forward — upstream's GPT-5.x prompting guide updates should be retained while keeping fork-added skills. Verify `codex-rs/skills/src/lib.rs` skill registration list after the merge.

### 13. `announcement_tip.toml` — Shared
- **Type:** Shared (file edited by both sides).
- **Description:** TUI startup announcement banner config.
- **Implementation:** `announcement_tip.toml`. Local replaces the upstream "Welcome to Codex!" + "BREAKING NEWS gpt-5.3-codex" entries with a single fork-specific "Update Required - This version will no longer be supported starting May 8th" tip pointing at `https://github.com/openai/codex/releases/latest` (regex matches versions `0.0.x..0.119.x`, expires `2026-05-08`).
- **Merge plan:** **Replace with fork content** (the announcement is policy, not upstream-driven). Today is 2026-05-08 — the fork tip's `to_date` already expired, so consider updating to a fresh Ata-targeted announcement (or removing entirely) instead of merging upstream's now-stale Codex announcements back in.

### 14. `docs/` Codex→Ata rebrand — Shared (mass URL/brand edits)
- **Type:** Shared.
- **Description:** Every shared doc swaps `developers.openai.com/codex/...` URLs for `https://github.com/Agents2AgentsAI/ata/blob/main/docs/...` and the brand "Codex" → "Ata".
- **Implementation:** `docs/agents_md.md`, `docs/authentication.md`, `docs/config.md`, `docs/example-config.md`, `docs/exec.md`, `docs/execpolicy.md`, `docs/getting-started.md`, `docs/install.md`, `docs/sandbox.md`, `docs/skills.md`, `docs/slash_commands.md`, `docs/CLA.md` (CLA party renamed OpenAI → Agents2Agents AI), `README.md`. `docs/install.md` also gains a `curl | sh` install one-liner. `docs/contributing.md` and `docs/open-source-fund.md` are deleted in fork.
- **Merge plan:** **Re-apply rebrand after each upstream sync** via the same sed-style transform: `Codex CLI → Ata CLI`, `Codex → Ata` in narrative copy, `~/.codex/ → ~/.ata/`, `developers.openai.com/codex/* → github.com/Agents2AgentsAI/ata/blob/main/docs/*`, `npm install -g @openai/codex → @a2a-ai/ata`, `git clone openai/codex → Agents2AgentsAI/ata`, `cargo run --bin codex → ata`. Re-add the curl-install block. Re-delete `docs/contributing.md` and `docs/open-source-fund.md` if upstream re-touches them.

### 15. Local-only fork docs (no upstream equivalent)
- **Type:** Local-only (~17 docs, ~2500 lines).
- **Description / files:**
  - Setup guides: `docs/paper-search-setup.md`, `docs/patent-search-setup.md`, `docs/zotero-setup.md` (+ `docs/images/zotero-local-api-settings.png`), `docs/lsp-treesitter-setup.md`, `docs/COORDINATION_SETUP.md` (multi-instance ATA relay), `docs/js_repl.md`.
  - Design notes: `docs/exit-confirmation-prompt-design.md`, `docs/browser-automation-findings.md`, `docs/tui-alternate-screen.md`, `docs/tui-chat-composer.md`, `docs/tui-request-user-input.md`, `docs/tui-stream-chunking-{review,tuning,validation}.md`, `docs/superpowers/plans/2026-03-20-alignment-driven-karaoke.md`, `docs/superpowers/plans/2026-03-20-tts-karaoke-sync-test.md`, `docs/prompts.md`.
  - Inside `codex-rs/`: `codex-rs/docs/superpowers/plans/2026-03-18-prompt-inspector.md`, `codex-rs/docs/superpowers/specs/2026-03-18-prompt-inspector-design.md`. Also `codex-rs/artifacts/README.md`, `codex-rs/package-manager/README.md`, `codex-rs/tools/prompt-inspector/README.md`, `codex-rs/tools/rollout-analyzer/README.md`, `codex-rs/utils/git/README.md`.
- **Merge plan:** **Keep all.** None overlap with upstream. They document fork-only features (research tooling, coordination, prompt inspector, package manager, alternate-screen TUI, karaoke TTS), so no upstream conflict is possible.

### 16. `UPSTREAM.md` — Local-only provenance
- **Type:** Local-only.
- **Description:** 11-line table tracking which Ata version is based on which Codex commit (v0.1.0 → 02e900654 … v0.3.3 → a7eda6a29) plus feature highlights per version.
- **Implementation:** `UPSTREAM.md`.
- **Merge plan:** **Update on every upstream sync.** Add a row pointing to `rust-v0.129.0` (or the SHA) with the merge date and a one-liner of new features, rather than just regenerating it.

### Cross-cutting observations
- The fork has fully replaced upstream's persona/personality system (`templates/personalities/gpt-5.2-codex_friendly.md` etc., `templates/collab/`, `templates/compact/`, `templates/goals/`, `templates/realtime/backend_prompt.md` are all absent locally) with the new `templates/collaboration_mode/` four-mode system, plus `templates/memories/` (consolidation/read_path/stage_one_input/stage_one_system) and `templates/research/`. **A merge that reintroduces upstream's `personalities/`, `collab/`, `compact/` directories will silently break the fork's collaboration-mode contract** — confirm those upstream dirs stay deleted post-merge.
- `protocol/src/prompts/permissions/**` and `protocol/src/prompts/realtime/**` are upstream additions absent locally; merging them in is fine (no fork content to preserve there).
- `codex-rs/core/templates/review/exit_success.xml`, `model_instructions/gpt-5.2-codex_instructions_template.md`, and `goals/{budget_limit,continuation}.md` have small textual diffs — verify each contains only minor brand rename (`Codex → Ata`) before accepting upstream changes.
- The Codex→Ata persona rename, `~/.codex/ → ~/.ata/` path swap, and `developers.openai.com/codex/* → github.com/Agents2AgentsAI/ata/*` URL swap appear ~30+ times across prompts and docs — the merge should be driven by a scripted post-merge sweep, not hand edits, to avoid leaving "Codex" remnants in agent-facing strings.
