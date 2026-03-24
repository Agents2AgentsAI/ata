# Prompt Inspector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a neovim-integrated tool for browsing all agent-facing content (prompts, instructions, tool descriptions, skills) with token counts, git metadata, and jump-to-source — plus a Rust CLI command to dump the fully assembled startup context.

**Architecture:** Python data backend discovers and extracts prompt content → JSON → Neovim plugin renders interactive tree browser + Telescope search. Separate Rust CLI command (`codex debug dump-initial-context`) dumps the real assembled startup context using the same building blocks as the agent. A TOML registry tracks inline Rust content that can't be auto-discovered.

**Tech Stack:** Python 3.10+ (no pip deps on 3.11+), Neovim Lua (lazy.nvim + Telescope), Rust (minimal additions to codex-core + codex-cli)

**Spec:** `docs/superpowers/specs/2026-03-18-prompt-inspector-design.md`

---

### Task 1: Create the prompt registry

The TOML file that tracks inline Rust agent-facing content that auto-discovery can't find.

**Files:**
- Create: `tools/prompt-inspector/prompt-registry.toml`

- [ ] **Step 1: Create the directory and registry file**

```bash
mkdir -p tools/prompt-inspector
```

Create `tools/prompt-inspector/prompt-registry.toml` with all ~40 entries. Each entry needs: `name`, `category`, `file` (relative to codex-rs/), `pattern` (const name or function name), optional `type = "function"`, and `description`.

The full registry must include these entries (grouped by category):

**category = "system-prompts":**
- `DEFAULT_PERSONALITY_HEADER` in `core/src/models_manager/model_info.rs`
- `LOCAL_FRIENDLY_TEMPLATE` in `core/src/models_manager/model_info.rs`
- `LOCAL_PRAGMATIC_TEMPLATE` in `core/src/models_manager/model_info.rs`

**category = "tool-descriptions":**
- `PRESENT_DOCUMENT_TOOL` in `core/src/tools/handlers/document_reader.rs`
- `UPDATE_DOCUMENT_SECTION_TOOL` in `core/src/tools/handlers/document_reader.rs`
- `APPEND_TO_SECTION_TOOL` in `core/src/tools/handlers/document_reader.rs`
- `PATCH_DOCUMENT_SECTION_TOOL` in `core/src/tools/handlers/document_reader.rs`
- `reading_view_display_mode_guidance` (function) in `core/src/tools/handlers/document_reader.rs`
- `ATTACH_URL_FILES_TOOL` in `core/src/tools/handlers/attach_url_files.rs`
- `PLAN_TOOL` in `core/src/tools/handlers/plan.rs`
- `create_shell_tool` (function) in `core/src/tools/spec.rs`
- `create_shell_command_tool` (function) in `core/src/tools/spec.rs`
- `create_exec_command_tool` (function) in `core/src/tools/spec.rs`
- `create_write_stdin_tool` (function) in `core/src/tools/spec.rs`
- `create_spawn_agent_tool` (function) in `core/src/tools/spec.rs`
- `create_send_input_tool` (function) in `core/src/tools/spec.rs`
- `create_resume_agent_tool` (function) in `core/src/tools/spec.rs`
- `create_wait_tool` (function) in `core/src/tools/spec.rs`
- `create_close_agent_tool` (function) in `core/src/tools/spec.rs`
- `create_team_post_tool` (function) in `core/src/tools/spec.rs`
- `request_user_input_tool_description` (function) in `core/src/tools/handlers/request_user_input.rs`
- `create_crop_and_store_figure_tool` (function) in `core/src/tools/spec/workspace.rs`
- `create_view_image_tool` (function) in `core/src/tools/spec/workspace.rs`
- `create_grep_files_tool` (function) in `core/src/tools/spec/workspace.rs`
- `create_read_file_tool` (function) in `core/src/tools/spec/workspace.rs`
- `create_list_dir_tool` (function) in `core/src/tools/spec/workspace.rs`
- `create_js_repl_tool` (function) in `core/src/tools/spec/javascript.rs`
- `create_artifacts_tool` (function) in `core/src/tools/spec/javascript.rs`
- `create_spawn_agents_on_csv_tool` (function) in `core/src/tools/spec/agent_jobs.rs`
- `create_apply_patch_freeform_tool` (function) in `core/src/tools/handlers/apply_patch.rs`
- `create_apply_patch_json_tool` (function) in `core/src/tools/handlers/apply_patch.rs`

**category = "agent-messages":**
- `FORKED_SPAWN_AGENT_OUTPUT_MESSAGE` in `core/src/agent/control.rs`
- `build_worker_prompt` (function) in `core/src/tools/handlers/agent_jobs.rs`
- `REALTIME_CONVERSATION_PROMPT` in `tui/src/chatwidget/realtime.rs`

**category = "context-injection":**
- `commit_message_trailer_instruction` (function) in `core/src/commit_attribution.rs`
- `render_js_repl_instructions` (function) in `core/src/project_doc.rs`
- `render_skills_section` (function) in `core/src/skills/render.rs`
- `render_apps_section` (function) in `core/src/apps/render.rs`
- `asking_questions_guidance_message` (function) in `core/src/models_manager/collaboration_mode_presets.rs`
- `serialize_to_xml` (function, EnvironmentContext) in `core/src/environment_context.rs`

- [ ] **Step 2: Verify registry entries are valid**

For each entry, open the referenced file and confirm the pattern exists:

```bash
# Spot-check a few entries
grep -n "PRESENT_DOCUMENT_TOOL" core/src/tools/handlers/document_reader.rs
grep -n "FORKED_SPAWN_AGENT_OUTPUT_MESSAGE" core/src/agent/control.rs
grep -n "create_shell_tool" core/src/tools/spec.rs
grep -n "REALTIME_CONVERSATION_PROMPT" tui/src/chatwidget/realtime.rs
```

Expected: each grep returns at least one line with a match.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/prompt-registry.toml
git commit -m "feat: add prompt inspector registry for inline Rust agent-facing content"
```

---

### Task 2: Python data backend

The discovery engine, extractor, and metadata enricher. Outputs JSON to stdout.

**Files:**
- Create: `tools/prompt-inspector/generate.py`
- Create: `tools/prompt-inspector/extractor.py`
- Create: `tools/prompt-inspector/metadata.py`

- [ ] **Step 1: Create `extractor.py`**

This module extracts string content from different source types.

Functions needed:
- `extract_file_content(filepath: str) -> str` — reads `.md`, `.txt`, `.xml`, `.toml` files entirely
- `extract_rust_const(filepath: str, pattern: str) -> tuple[str, int]` — finds `pattern` in a `.rs` file, extracts the string literal value from the `description:` field or const assignment. Returns `(content, line_number)`. Must handle:
  - `"..."` and `r#"..."#` raw strings
  - Multi-line strings (continued with `\n\`)
  - `concat!()` and `format!()` (extract the template string)
  - `LazyLock<ToolSpec>` structs — find the `description:` field within the struct
- `extract_rust_function(filepath: str, pattern: str) -> tuple[str, int]` — finds `fn {pattern}` in a `.rs` file, extracts the full function body (from `fn` to its matching closing brace). Returns `(source_code, line_number)`.

Error handling: if extraction fails, return `(None, 0)` rather than raising.

- [ ] **Step 2: Verify extractor works on a few real files**

```bash
cd /Users/nima/a2a/codex/codex-rs
python3 -c "
from tools.prompt_inspector.extractor import extract_file_content, extract_rust_const, extract_rust_function
# Test file extraction
content = extract_file_content('core/templates/compact/prompt.md')
print(f'File: {len(content)} bytes')
# Test const extraction
content, line = extract_rust_const('core/src/agent/control.rs', 'FORKED_SPAWN_AGENT_OUTPUT_MESSAGE')
print(f'Const: line {line}, {len(content)} bytes')
# Test function extraction
content, line = extract_rust_function('core/src/commit_attribution.rs', 'commit_message_trailer_instruction')
print(f'Function: line {line}, {len(content)} bytes')
"
```

Expected: all three print non-zero byte counts.

Note: the Python files are standalone scripts, not a package. Use `sys.path.insert` or run them directly. Do NOT create `__init__.py` files — the scripts import each other via relative imports within the same directory. The verification script should be:

```bash
cd /Users/nima/a2a/codex/codex-rs
PYTHONPATH=tools/prompt-inspector python3 -c "
from extractor import extract_file_content, extract_rust_const, extract_rust_function
..."
```

- [ ] **Step 3: Create `metadata.py`**

Functions needed:
- `token_count(text: str) -> int` — returns `math.ceil(len(text.encode('utf-8')) / 4)`
- `git_metadata(filepath: str, codex_root: str) -> dict` — runs `git log -1 --format="%ar|%an|%ai" -- {filepath}` from `codex_root`, returns `{"modified": "3 days ago", "author": "alice", "date": "2026-03-15T10:30:00+00:00"}`. Returns empty dict on failure.
- `git_metadata_batch(filepaths: list[str], codex_root: str) -> dict[str, dict]` — runs git log for all files in parallel using `concurrent.futures.ThreadPoolExecutor(max_workers=16)`. Returns dict mapping filepath → metadata dict.

- [ ] **Step 4: Create `generate.py`**

The main entry point. Accepts CLI args:
- `--codex-root PATH` (required) — path to `codex-rs/`
- `--metadata-only` — output JSON without `content` fields (fast mode for tree rendering)
- `--entry NAME` — output JSON for a single entry with full content (lazy loading)

Logic:
1. Load `prompt-registry.toml` from `{codex_root}/tools/prompt-inspector/prompt-registry.toml`
2. Auto-discover files via glob patterns (see spec for full list)
3. Grep `.rs` files for `include_str!` calls, resolve paths, add any not already discovered
4. Also glob `core/gpt*.md` for model-specific prompt files (e.g., `core/gpt_5_codex_prompt.md`, `core/gpt_5_2_prompt.md`, etc.) → category "System Prompts"
5. Map auto-discovered files to categories based on their directory:
   - `core/templates/collaboration_mode/` → "Collaboration"
   - `core/templates/memories/` → "Memory System"
   - `core/templates/research/` → "Research"
   - `core/templates/review/` → "Review"
   - `core/templates/compact/` → "Compact"
   - `core/templates/coordination/` → "Coordination"
   - `core/templates/agents/` → "Agent Messages"
   - `core/templates/collab/` → "Collaboration"
   - `core/templates/tools/` → "Tool Descriptions"
   - `core/templates/search_tool/` → "Tool Descriptions"
   - `core/templates/model_instructions/` → "System Prompts"
   - `core/templates/personalities/` → "Personalities"
   - `protocol/src/prompts/permissions/` → "Permissions"
   - `protocol/src/prompts/realtime/` → "Realtime"
   - `protocol/src/prompts/base_instructions/` → "System Prompts"
   - `skills/src/assets/**/SKILL.md` → "Skills"
   - `skills/src/assets/**/references/` → "Skill References"
   - `core/src/tools/handlers/tool_*.txt` → "Tool Descriptions"
   - `core/src/agent/builtins/` → "Agent Messages"
   - Root-level `.md` files → "System Prompts"
5. For each entry: extract content (unless `--metadata-only`), compute token count, collect git metadata
6. Group by category, compute category totals
7. Output JSON to stdout

- [ ] **Step 5: Verify generate.py produces valid output**

```bash
cd /Users/nima/a2a/codex/codex-rs
python3 tools/prompt-inspector/generate.py --codex-root . --metadata-only | python3 -m json.tool | head -40
```

Expected: valid JSON with categories array, entries with names/tokens/sources, no content fields.

```bash
python3 tools/prompt-inspector/generate.py --codex-root . --entry "base instructions" | python3 -m json.tool | head -20
```

Expected: single entry with full content included.

- [ ] **Step 6: Create `requirements.txt`**

Create `tools/prompt-inspector/requirements.txt`:
```
tomli>=1.1.0 ; python_version < "3.11"
```

(On Python 3.11+, `tomllib` is in stdlib. No pip install needed.)

- [ ] **Step 7: Commit**

```bash
git add tools/prompt-inspector/generate.py tools/prompt-inspector/extractor.py tools/prompt-inspector/metadata.py tools/prompt-inspector/requirements.txt
git commit -m "feat: add prompt inspector Python data backend"
```

---

### Task 3: Python validator

Drift detection script for `just check-prompts`.

**Files:**
- Create: `tools/prompt-inspector/validate.py`

- [ ] **Step 1: Create `validate.py`**

Accepts:
- `--codex-root PATH` (default: auto-detect from script location, i.e., `../../` from the script's directory)

Three checks:

**Check 1 — `include_str!` coverage:**
- Grep all `.rs` files under `codex_root` for `include_str!("...")` calls
- Resolve the relative path from the `.rs` file's location
- Skip paths containing `test`, `fixture`, `snapshot` (test-only includes)
- For each resolved file: check if it matches any auto-discovery glob pattern. If not, warn.

**Check 2 — `@agent-facing` coverage:**
- Grep all `.rs` files for `// @agent-facing` comments
- For each, find the next non-comment line to get the const/fn name
- Check if there's a matching entry in `prompt-registry.toml` (match on `file` + `pattern`)
- If not, warn.

**Check 3 — Registry integrity:**
- For each entry in `prompt-registry.toml`:
  - Check the file exists at `codex_root / entry.file`
  - Grep for `entry.pattern` in the file
  - If file missing or pattern not found, error.

Output: formatted report with warnings and errors. Exit code 0 if clean, 1 otherwise.

- [ ] **Step 2: Verify validator runs cleanly**

```bash
cd /Users/nima/a2a/codex/codex-rs
python3 tools/prompt-inspector/validate.py --codex-root .
echo $?
```

Expected: exit code 0 (all registry entries valid, no `@agent-facing` annotations added yet so check 2 has nothing to flag). There may be warnings for `include_str!` calls not yet categorized — that's expected and can be triaged.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/validate.py
git commit -m "feat: add prompt inspector validator for drift detection"
```

---

### Task 4: Add `@agent-facing` annotations to Rust source

Add `// @agent-facing` comments above each inline agent-facing constant/function in Rust. These are comment-only changes.

**Files:**
- Modify: every file referenced in `prompt-registry.toml` entries

- [ ] **Step 1: Add annotations to tool handler constants**

Add `// @agent-facing` on the line directly above each of these constants/functions:

In `core/src/tools/handlers/document_reader.rs`:
- `PRESENT_DOCUMENT_TOOL`
- `UPDATE_DOCUMENT_SECTION_TOOL`
- `APPEND_TO_SECTION_TOOL`
- `PATCH_DOCUMENT_SECTION_TOOL`
- `fn reading_view_display_mode_guidance`

In `core/src/tools/handlers/attach_url_files.rs`:
- `ATTACH_URL_FILES_TOOL`

In `core/src/tools/handlers/plan.rs`:
- `PLAN_TOOL`

In `core/src/tools/handlers/request_user_input.rs`:
- `fn request_user_input_tool_description`

In `core/src/tools/handlers/apply_patch.rs`:
- `fn create_apply_patch_freeform_tool`
- `fn create_apply_patch_json_tool`

In `core/src/tools/handlers/agent_jobs.rs`:
- `fn build_worker_prompt`

- [ ] **Step 2: Add annotations to tool spec builders**

In `core/src/tools/spec.rs`:
- `fn create_shell_tool`
- `fn create_shell_command_tool`
- `fn create_exec_command_tool`
- `fn create_write_stdin_tool`
- `fn create_spawn_agent_tool`
- `fn create_send_input_tool`
- `fn create_resume_agent_tool`
- `fn create_wait_tool`
- `fn create_close_agent_tool`
- `fn create_team_post_tool`

In `core/src/tools/spec/workspace.rs`:
- `fn create_crop_and_store_figure_tool`
- `fn create_view_image_tool`
- `fn create_grep_files_tool`
- `fn create_read_file_tool`
- `fn create_list_dir_tool`

In `core/src/tools/spec/javascript.rs`:
- `fn create_js_repl_tool`
- `fn create_artifacts_tool`

In `core/src/tools/spec/agent_jobs.rs`:
- `fn create_spawn_agents_on_csv_tool`

- [ ] **Step 3: Add annotations to prompt/instruction code**

In `core/src/models_manager/model_info.rs`:
- `DEFAULT_PERSONALITY_HEADER`
- `LOCAL_FRIENDLY_TEMPLATE`
- `LOCAL_PRAGMATIC_TEMPLATE`

In `core/src/agent/control.rs`:
- `FORKED_SPAWN_AGENT_OUTPUT_MESSAGE`

In `core/src/commit_attribution.rs`:
- `fn commit_message_trailer_instruction`

In `core/src/project_doc.rs`:
- `fn render_js_repl_instructions`

In `core/src/skills/render.rs`:
- `fn render_skills_section`

In `core/src/apps/render.rs`:
- `fn render_apps_section`

In `core/src/models_manager/collaboration_mode_presets.rs`:
- `fn asking_questions_guidance_message`

In `core/src/environment_context.rs`:
- `fn serialize_to_xml`

In `tui/src/chatwidget/realtime.rs`:
- `REALTIME_CONVERSATION_PROMPT`

- [ ] **Step 4: Verify validator catches all annotations**

```bash
cd /Users/nima/a2a/codex/codex-rs
python3 tools/prompt-inspector/validate.py --codex-root .
echo $?
```

Expected: exit code 0. All `@agent-facing` annotations match registry entries.

- [ ] **Step 5: Commit**

Stage only the files that were annotated (do NOT use `git add -u` as the working tree has unrelated modified files):

```bash
git add \
  core/src/tools/handlers/document_reader.rs \
  core/src/tools/handlers/attach_url_files.rs \
  core/src/tools/handlers/plan.rs \
  core/src/tools/handlers/request_user_input.rs \
  core/src/tools/handlers/apply_patch.rs \
  core/src/tools/handlers/agent_jobs.rs \
  core/src/tools/spec.rs \
  core/src/tools/spec/workspace.rs \
  core/src/tools/spec/javascript.rs \
  core/src/tools/spec/agent_jobs.rs \
  core/src/models_manager/model_info.rs \
  core/src/agent/control.rs \
  core/src/commit_attribution.rs \
  core/src/project_doc.rs \
  core/src/skills/render.rs \
  core/src/apps/render.rs \
  core/src/models_manager/collaboration_mode_presets.rs \
  core/src/environment_context.rs \
  tui/src/chatwidget/realtime.rs
git commit -m "feat: add @agent-facing annotations to all inline agent-facing Rust content"
```

---

### Task 5: Neovim plugin — data layer

The Lua module that shells out to `generate.py` and manages caching.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/data.lua`

- [ ] **Step 1: Create directory structure**

```bash
mkdir -p tools/prompt-inspector/plugin/lua/prompt-inspector
```

- [ ] **Step 2: Create `data.lua`**

This module:
- Exposes `M.load_metadata(codex_root, callback)` — async, shells out to `python3 generate.py --codex-root {root} --metadata-only`, parses JSON, calls `callback(data)`
- Exposes `M.load_entry(codex_root, entry_name, callback)` — async, shells out to `python3 generate.py --codex-root {root} --entry {name}`, parses JSON, calls `callback(entry)`
- Exposes `M.refresh(codex_root, callback)` — forces regeneration, same as `load_metadata` but bypasses cache
- Caching: stores parsed metadata in `M._cache`. On `load_metadata`, computes a hash of registry mtime + script mtime. If hash matches `M._cache_hash`, returns cached data immediately. Otherwise regenerates.
- Uses `vim.fn.jobstart()` for async subprocess execution with `on_stdout`/`on_exit` callbacks
- Resolves `generate.py` path from plugin directory: `vim.fn.fnamemodify(debug.getinfo(1).source:sub(2), ":h:h:h:h") .. "/generate.py"`
- On first call, checks that `python3` is available via `vim.fn.executable("python3")`. If not, shows `vim.notify("prompt-inspector: python3 not found in PATH", vim.log.levels.ERROR)` and returns early

- [ ] **Step 3: Verify data loads in neovim**

Open neovim and run:
```vim
:lua local d = require("prompt-inspector.data"); d.load_metadata("/Users/nima/a2a/codex/codex-rs", function(data) print(vim.inspect(data.total_tokens)) end)
```

Expected: prints a number (the total token count).

- [ ] **Step 4: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/data.lua
git commit -m "feat: add prompt inspector neovim data layer"
```

---

### Task 6: Neovim plugin — tree browser

The interactive tree buffer with categories, entries, token counts, and navigation.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/tree.lua`

- [ ] **Step 1: Create `tree.lua`**

This module manages the tree buffer (left pane). Key elements:

**State:**
- `M._buf` — the tree buffer number
- `M._win` — the tree window number
- `M._data` — loaded metadata (categories + entries)
- `M._expanded` — set of expanded category names
- `M._cursor_entry` — currently selected entry (for preview updates)
- `M._sort_mode` — current sort: "name" | "tokens" | "date"
- `M._filter` — current filter string or nil

**Buffer rendering (`M._render()`):**
- Scratch buffer, `buftype=nofile`, `filetype=prompt-inspector`
- `modifiable=false` except during render
- Each line is either a category header or an entry:
  - Category (collapsed): `► Category Name            1,200t  (5)`
  - Category (expanded): `▼ Category Name            1,200t  (5)`
  - Entry: `  ● entry name               340t`
  - Entry with error: `  ✗ entry name (broken)        0t` (highlighted in red)
- Footer line: `Total: ~{total_tokens} tokens across {count} entries`

**Keybindings** (set via `vim.keymap.set("n", key, fn, { buffer = buf })`):
- `j/k` — navigate (standard vim, no custom mapping needed)
- `<CR>` — on category line: toggle expand/collapse. On entry line: open source in vsplit via `vim.cmd("vsplit " .. source_path)` then `vim.cmd(":" .. line_number)`
- `e` — open source file at line in vertical split
- `o` — open source file at line in current window (close tree first)
- `s` — cycle sort mode, re-render
- `/` — prompt for filter string via `vim.fn.input("Filter: ")`, re-render showing only matching entries
- `R` — call `data.refresh()`, re-render
- `T` — toggle showing/hiding token counts
- `q` — close inspector (close tree window + preview window)
- `?` — show help in a floating window

**Preview integration:**
- On `CursorMoved` autocmd, check if cursor is on an entry line
- If so, call `preview.show(entry)` to update the preview buffer
- Uses `data.load_entry()` to lazily load content if not already cached

**Window setup:**
- Tree window: `width=36`, left side, `winfixwidth=true`
- Highlight groups: `PromptInspectorCategory`, `PromptInspectorEntry`, `PromptInspectorTokens`, `PromptInspectorBroken`

- [ ] **Step 2: Verify tree renders**

Open neovim, manually call:
```vim
:lua require("prompt-inspector.data").load_metadata("/Users/nima/a2a/codex/codex-rs", function(data) require("prompt-inspector.tree").open(data) end)
```

Expected: a tree buffer appears on the left with categories and entries. Pressing `<CR>` on a category toggles it.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/tree.lua
git commit -m "feat: add prompt inspector tree browser"
```

---

### Task 7: Neovim plugin — preview pane

The right-side buffer that shows prompt content with metadata header.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/preview.lua`

- [ ] **Step 1: Create `preview.lua`**

This module manages the preview buffer (right pane).

**Functions:**
- `M.open()` — creates the preview buffer and window (right of tree, takes remaining space)
- `M.show(entry, codex_root)` — loads entry content (via `data.load_entry` if not cached), renders:
  - Line 1: `# {entry.name}` (highlighted as markdown heading)
  - Line 2: `Source: {entry.source}` (the `file:line` path — `gF` jumps here natively)
  - Line 3: `Tokens: {entry.tokens}  |  Modified: {entry.git_modified}  |  By: {entry.git_author}`
  - Line 4: blank
  - Line 5+: the full content
- Buffer is `modifiable=false`, `buftype=nofile`
- Filetype set based on source file extension:
  - `.md` → `markdown`
  - `.toml` → `toml`
  - `.rs` (const/function extraction) → `rust`
  - `.xml` → `xml`
  - `.txt` → `text`
- `q` keybinding closes the inspector
- `e` keybinding opens the source file (same as tree's `e`)

- [ ] **Step 2: Verify preview shows content**

With the tree already open from Task 6 verification, navigate to an entry and check the preview updates.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/preview.lua
git commit -m "feat: add prompt inspector preview pane"
```

---

### Task 8: Neovim plugin — Telescope extension

Fuzzy search across all prompt entries.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/telescope.lua`

- [ ] **Step 1: Create `telescope.lua`**

Uses Telescope's `pickers.new()` API.

**Picker setup:**
- Results: flat list of all entries across all categories
- Each result displays: `[Category] entry name  (340t)`
- Ordinal (for fuzzy matching): `category name description content` (if content is cached)
- Previewer: custom previewer that shows the same header + content as the preview pane
- Actions:
  - `<CR>` (select_default): open source file at line in current window
  - `<C-v>`: open source file at line in vertical split
  - `<C-q>`: send all results to quickfix list

**Function:**
- `M.search(codex_root)` — loads metadata, creates and opens the picker

- [ ] **Step 2: Verify telescope picker works**

```vim
:lua require("prompt-inspector.telescope").search("/Users/nima/a2a/codex/codex-rs")
```

Expected: Telescope picker opens, entries are searchable, preview shows content.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/telescope.lua
git commit -m "feat: add prompt inspector Telescope search"
```

---

### Task 9: Neovim plugin — context viewer

Shows the full assembled startup context from the Rust dump command.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/context.lua`

- [ ] **Step 1: Create `context.lua`**

**Function:**
- `M.show(codex_root)` — runs `cargo run -p codex-cli -- debug dump-initial-context` (from `codex_root`), captures stdout, opens in a new buffer with:
  - `buftype=nofile`
  - `filetype=markdown`
  - `modifiable=false`
  - `q` to close
  - Buffer name: `[Prompt Context]`
  - Uses `vim.fn.jobstart()` for async execution
  - Shows `Loading context...` message while running
  - On error (non-zero exit or cargo not found), shows error message in the buffer

Note: this depends on Task 11 (Rust dump command) being implemented. Until then, the command will fail gracefully with an error message in the buffer ("cargo command failed — run Task 11 first"). Task 9 is listed before Task 11 in the plan because it completes the plugin module set, but its full verification is deferred until after Task 11.

- [ ] **Step 2: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/context.lua
git commit -m "feat: add prompt inspector context viewer"
```

---

### Task 10: Neovim plugin — init and setup

The main entry point that wires everything together.

**Files:**
- Create: `tools/prompt-inspector/plugin/lua/prompt-inspector/init.lua`

- [ ] **Step 1: Create `init.lua`**

**`M.setup(opts)` function:**
- Validates `opts.codex_root` (required, must be a directory)
- Expands `~` in path via `vim.fn.expand()`
- Stores config in `M._config`
- Creates user commands:
  - `:PromptInspector` → calls `M.open()`
  - `:PromptSearch` → calls `telescope.search(codex_root)`
  - `:PromptContext` → calls `context.show(codex_root)`
  - `:PromptRefresh` → calls `data.refresh()` then re-renders tree
- Registers keybindings (only if not already bound):
  - `<leader>ip` → `:PromptInspector`
  - `<leader>is` → `:PromptSearch`
  - `<leader>if` → `:PromptContext`
- Registers which-key group (if which-key is available):
  ```lua
  local ok, wk = pcall(require, "which-key")
  if ok then
    wk.add({ { "<leader>i", group = "inspect" } })
  end
  ```

**`M.open()` function:**
1. Calls `data.load_metadata(codex_root, function(data) ... end)`
2. In callback: calls `tree.open(data)` then `preview.open()`
3. Sets focus to tree window

- [ ] **Step 2: Test full plugin flow**

Add to nvim config temporarily (or source directly):
```vim
:set runtimepath+=~/a2a/codex/codex-rs/tools/prompt-inspector/plugin
:lua require("prompt-inspector").setup({ codex_root = "~/a2a/codex/codex-rs" })
:PromptInspector
```

Expected: tree browser opens on left, preview on right, navigation works, `e` jumps to source, `s` sorts, `/` filters.

- [ ] **Step 3: Commit**

```bash
git add tools/prompt-inspector/plugin/lua/prompt-inspector/init.lua
git commit -m "feat: add prompt inspector plugin init and setup"
```

---

### Task 11: Rust dump-initial-context command

A public function in `codex-core` that assembles the initial context, plus a CLI subcommand that calls it.

**Files:**
- Create: `core/src/dump_context.rs`
- Modify: `core/src/lib.rs` (add `pub mod dump_context;`)
- Modify: `cli/src/main.rs` (add debug subcommand variant + dispatch)

- [ ] **Step 1: Create `core/src/dump_context.rs`**

This module lives inside `codex-core` (so it can access all the `pub(crate)` building blocks without visibility changes). It exports a public function + result struct.

**Key references to follow:**
- `core/src/codex.rs:3165-3289` — `Session::build_initial_context()` is the source of truth for how the context is assembled. Follow its logic section by section.
- `core/src/codex.rs:443-452` — how `base_instructions` is resolved (config override > model default with personality)
- `cli/src/main.rs:1131-1170` — `run_debug_clear_memories_command()` shows the pattern for loading config in a debug subcommand

**Struct to export:**
```rust
pub struct DumpResult {
    pub base_instructions: String,
    pub developer_message: String,
    pub user_message: String,
    pub tools_summary: String,
    pub tools_count: usize,
}
```

Plus `pub fn format_dump_result(result: &DumpResult) -> String` that formats the output with section headers, token counts (`ceil(bytes/4)`), and a total summary (see spec for exact format).

**Assembly logic (pseudo-code — follow the actual APIs in `build_initial_context()`):**

```
1. Resolve model info:
   - model_info_from_slug(config.model or default)
   - with_config_overrides(model_info, config)

2. Base instructions:
   - config.base_instructions OR model_info.get_model_instructions(personality)

3. Developer message (concatenate these sections):
   a. DeveloperInstructions::from_policy(sandbox_policy, approval_policy, exec_policy, cwd, request_permission_enabled)
      - sandbox_policy: from config.permissions.sandbox_policy (Constrained<SandboxPolicy>)
      - approval_policy: from config.permissions.approval_policy (Constrained<AskForApproval>)
      - exec_policy: load via load_exec_policy() or use default Policy
   b. config.developer_instructions (if set)
   c. Collaboration mode: builtin_collaboration_mode_presets(CollaborationModesConfig{...})
      → DeveloperInstructions::from_collaboration_mode(mode) for the default mode
   d. commit_message_trailer_instruction(config.commit_attribution.as_deref())

4. User message:
   - get_user_instructions(config, skills, None).await
   - EnvironmentContext XML (construct manually with cwd, shell info, date, timezone)

5. Tools: "Tool listing requires a running session. Use the prompt inspector tree browser to see individual tool descriptions."
   (Tools depend on MCP servers, session state, etc. — not feasible to enumerate without a full session)
```

**Visibility: NO changes needed.** Because `dump_context.rs` lives inside `codex-core`, it can call all `pub(crate)` functions directly. Only `DumpResult` and `format_dump_result` need to be `pub` (they're new types, no conflict).

**Dependencies to check in `core/Cargo.toml`:**
- The module needs date/time formatting. Check if `chrono` or `time` is already a dependency. If not, use `std::time` or the existing date utilities in the codebase (search for how `current_date` is set in `build_initial_context()`).
- The `Shell` struct requires specific construction — look at how `Session` creates its shell (search for `user_shell` or `Shell::new` in `core/src/`).

- [ ] **Step 2: Add module to `core/src/lib.rs`**

Add `pub mod dump_context;` to the module declarations in `core/src/lib.rs`.

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p codex-core
```

Expected: compiles without errors. Fix any import/visibility issues.

- [ ] **Step 4: Add CLI subcommand to `cli/src/main.rs`**

Add to `DebugSubcommand` enum (around line 190):
```rust
/// Dump the fully assembled initial context that the agent receives.
DumpInitialContext,
```

Add dispatch in the match arm (around line 844):
```rust
DebugSubcommand::DumpInitialContext => {
    run_debug_dump_initial_context(&root_config_overrides, &interactive).await?;
}
```

Add the handler function. **Follow the exact pattern from `run_debug_clear_memories_command` at line 1131** for config loading:

```rust
async fn run_debug_dump_initial_context(
    root_config_overrides: &CliConfigOverrides,
    interactive: &TuiCli,
) -> anyhow::Result<()> {
    // Follow the same config loading pattern as run_debug_clear_memories_command
    let parsed_overrides = root_config_overrides.parse_overrides()?;
    let config = Config::load_with_cli_overrides_and_harness_overrides(
        parsed_overrides,
        ConfigOverrides {
            config_profile: interactive.config_profile.clone(),
            ..Default::default()
        },
    )?;
    let result = codex_core::dump_context::dump_initial_context(&config).await?;
    print!("{}", codex_core::dump_context::format_dump_result(&result));
    Ok(())
}
```

**Note:** verify the exact argument order by reading `run_debug_clear_memories_command` — the plan's ordering here matches that pattern but the actual code may differ slightly.

- [ ] **Step 5: Verify CLI command works**

```bash
cargo run -p codex-cli -- debug dump-initial-context 2>/dev/null | head -30
```

Expected: prints the base instructions section header and content.

- [ ] **Step 6: Commit**

```bash
git add core/src/dump_context.rs core/src/lib.rs cli/src/main.rs
git commit -m "feat: add codex debug dump-initial-context command"
```

---

### Task 12: Justfile recipes and CLAUDE.md updates

Wire everything together with `just` commands and document the conventions.

**Files:**
- Modify: `/Users/nima/a2a/codex/justfile`
- Modify: `/Users/nima/a2a/codex/codex-rs/CLAUDE.md`

- [ ] **Step 1: Add `just` recipes**

Add to the justfile (at the end, before any trailing content):

```just
# Launch prompt inspector in neovim
prompts:
    nvim -c "PromptInspector"

# Validate prompt registry and @agent-facing annotations
check-prompts:
    python3 tools/prompt-inspector/validate.py --codex-root .

# Dump full assembled agent context to terminal
dump-context:
    cargo run -p codex-cli -- debug dump-initial-context
```

- [ ] **Step 2: Update CLAUDE.md**

Add the following section to `/Users/nima/a2a/codex/codex-rs/CLAUDE.md`:

```markdown
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
```

- [ ] **Step 3: Add lazy.nvim spec to user's nvim config**

Add to `/Users/nima/.config/nvim/init.lua` inside the `lazy.setup({...})` plugins table:

```lua
{
  dir = "~/a2a/codex/codex-rs/tools/prompt-inspector/plugin",
  cmd = { "PromptInspector", "PromptSearch", "PromptContext" },
  keys = {
    { "<leader>ip", "<cmd>PromptInspector<cr>", desc = "Prompt Inspector" },
    { "<leader>is", "<cmd>PromptSearch<cr>", desc = "Search Prompts" },
    { "<leader>if", "<cmd>PromptContext<cr>", desc = "Full Agent Context" },
  },
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function()
    require("prompt-inspector").setup({
      codex_root = "~/a2a/codex/codex-rs",
    })
  end,
},
```

- [ ] **Step 4: End-to-end verification**

```bash
# Verify just recipes work
just check-prompts
just dump-context | head -10

# Verify neovim plugin loads
nvim -c "PromptInspector" -c "sleep 3" -c "qa"
```

- [ ] **Step 5: Commit**

```bash
git add justfile codex-rs/CLAUDE.md
git commit -m "feat: add prompt inspector just recipes and CLAUDE.md documentation"
```

---

## Task Dependency Graph

```
Task 1 (Registry) ─────────────────────┐
  ↓                                     │
Task 2 (Python Backend)                 │
  ↓                                     │
Task 3 (Validator)                      │
  ↓                                     │
Task 4 (@agent-facing)                  │
  ↓                                     │
Task 5 (Data Layer)        Task 11 (Rust Dump) ← INDEPENDENT, can be
  ↓                            ↓                   done in parallel with
Task 6 (Tree Browser)         │                   Tasks 5-10
  ↓                            │
Task 7 (Preview)               │
  ↓                            │
Task 8 (Telescope)             │
  ↓                            │
Task 9 (Context Viewer) ←─────┘ (needs Task 11 for full functionality)
  ↓
Task 10 (Init/Setup)
  ↓
Task 12 (Integration) ← depends on everything
```

**Parallel execution:** Tasks 5-10 (neovim plugin) and Task 11 (Rust dump) are independent and can be dispatched to separate agents. Task 9's context viewer will show a graceful error until Task 11 is complete.
