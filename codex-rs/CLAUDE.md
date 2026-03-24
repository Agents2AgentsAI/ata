Note that we merge upstream branch to our branches a lot, but we don't merge out branches to upstream. So we want to make sure we reduce the conflict surface.

## Release Branch & Public/Private Separation

The private `codex` repo has a `release` branch that is the **only** branch pushed to the public `ata` repo. Private `main` contains all code (public + private). The `release` branch has private code stripped and a clean history with no links to private main.

**Flow:** `upstream → codex main → release branch → ata public main`

**Push to public:** `git push public release:main`

**Sync main → release:** `just sync-release` (copies files from main, strips private dirs, restores cleaned mixed files, verifies compilation, commits — NO git merge, so no history leaks)

### Private code (must NEVER go on release)
- `codex-rs/coordination/` and `coordination-relay/` — coordination crates
- `codex-rs/supabase/` — Edge Functions, migrations, backend config
- `codex-rs/skills/src/assets/remote-exec/` — fleet/experiment skill
- `codex-rs/core/src/coordination_context.rs` and `core/src/tools/handlers/team_post.rs`
- Fleet worker code in `exec/src/lib.rs` (behind `#[cfg(feature = "relay")]`)

### Rule for new private features
**Always put new private code in its own crate/directory.** Never add private code inside shared crates (`core/`, `tui/`, `exec/`, `cli/`). This keeps `just sync-release` working without manual edits.

### Mixed files
Some files exist on both branches but differ (coordination/fleet refs stripped on release). These are listed in `_release_mixed_files` in the Justfile. If you add private code to a shared crate (which you shouldn't), you must add the file to that list.

## Agent-Facing Content

All content shown to the LLM agent (prompts, instructions, tool descriptions) is tracked
by the Prompt Inspector (`just prompts`).

### When adding new agent-facing content:

- **New template file** (`.md`/`.txt` in `core/templates/`, `protocol/src/prompts/`, etc.):
  Auto-discovered, no action needed.

- **New inline string in Rust** (tool description, prompt constant, dynamic builder):
  1. Add `// @agent-facing` comment above the const/function
  2. Add an entry to `tools/prompt-inspector/prompt-registry.toml`
  3. Run `just check-prompts` to verify

- **New skill file**: Place `SKILL.md` under `skills/src/assets/`. Auto-discovered.

### Validation

Run `just check-prompts` before pushing. This verifies all `include_str!` references
and `@agent-facing` annotations have corresponding entries in the inspector.
