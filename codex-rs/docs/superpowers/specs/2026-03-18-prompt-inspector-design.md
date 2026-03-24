# Prompt Inspector — Design Spec

## Purpose

A tool for browsing, searching, and understanding all agent-facing content in the ATA/Codex codebase: system prompts, instructions, tool descriptions, tool response patterns, skill files, and the fully assembled startup context. Shows token counts, git metadata, and lets you jump to source — all from inside neovim.

## Problem

The codebase has ~70+ files and inline code locations that contribute to what the LLM agent sees. These are scattered across template `.md` files, protocol prompts, skill files, inline Rust string constants, tool description builders, and dynamic prompt assembly functions. There is no way to:

- See them all in one place
- Know how many tokens each consumes
- Track when they were last changed and by whom
- See the fully assembled message the agent receives on startup
- Find inefficiencies or redundancies across prompt surfaces

## Components

### 1. Python data backend (`tools/prompt-inspector/generate.py`)

Discovers, extracts, and enriches all agent-facing content. Outputs structured JSON to stdout.

**Auto-discovery (zero maintenance):**

| Glob pattern | Category |
|---|---|
| `core/templates/**/*.md` | Varies by subdirectory (collaboration, memory, research, review, compact, coordination, agents, tools) |
| `protocol/src/prompts/**/*.md` | Permissions, sandbox, realtime, base instructions |
| `skills/src/assets/**/SKILL.md` | Skills |
| `skills/src/assets/**/references/*.md` | Skill references |
| `core/src/tools/handlers/tool_*.txt` | Tool description files |
| `core/src/agent/builtins/*.toml` | Agent role configs |
| `core/templates/**/*.xml` | Review exit templates |
| `apply-patch/*.md` | Apply patch instructions |
| Root-level: `core/prompt.md`, `core/prompt_with_apply_patch_instructions.md`, `core/review_prompt.md`, `core/hierarchical_agents_message.md`, `tui/prompt_for_init_command.md` | System prompts |

Plus: grep all `.rs` files for `include_str!` calls, resolve the referenced file path, add if not already in glob results.

**Registry (`tools/prompt-inspector/prompt-registry.toml`) for inline Rust content (~40 entries):**

Covers content that lives as inline string literals or dynamic builders in Rust source:

- Tool descriptions built in `core/src/tools/spec.rs` (`create_shell_tool`, `create_spawn_agent_tool`, etc.)
- Tool descriptions as constants in handlers (`PRESENT_DOCUMENT_TOOL`, `ATTACH_URL_FILES_TOOL`, `PLAN_TOOL`, etc.)
- Response formatting patterns in `core/src/tools/handlers/*.rs`
- Dynamic prompt builders (`build_research_prompt()`, `render_js_repl_instructions()`, `render_skills_section()`, `render_apps_section()`, `asking_questions_guidance_message()`)
- Inline constants (`DEFAULT_PERSONALITY_HEADER`, `LOCAL_FRIENDLY_TEMPLATE`, `LOCAL_PRAGMATIC_TEMPLATE`, `FORKED_SPAWN_AGENT_OUTPUT_MESSAGE`, `REALTIME_CONVERSATION_PROMPT`)
- Config-injected instructions (`commit_message_trailer_instruction()`, environment context XML template)

Registry entry format:

```toml
[[entry]]
name = "Reading View - present_document"
category = "tool-descriptions"
file = "core/src/tools/handlers/document_reader.rs"
pattern = "PRESENT_DOCUMENT_TOOL"
description = "Full description shown to agent for the present_document tool"

[[entry]]
name = "Research prompt builder"
category = "research"
file = "core/src/research/prompt.rs"
pattern = "build_research_prompt"
type = "function"  # indicates this is a function body, not a string constant
description = "Dynamic multi-phase research task prompt builder"
```

**Extraction logic (`tools/prompt-inspector/extractor.py`):**

For each entry (auto-discovered or registry):
- **`.md` / `.txt` files**: read full content directly
- **String literal constants** (registry, `type` omitted or `"const"`): find the pattern in the `.rs` file, then extract the `description:` field value from within the struct literal. Many tool descriptions live inside `LazyLock<ToolSpec>` or `ToolSpec { ... }` structs — the extractor locates the `description:` field and extracts its string value (handles `"..."`, `r#"..."#`, multi-line, `format!()`, and `concat!()`)
- **Function bodies** (registry, `type = "function"`): extract the function source from `fn {pattern}` to closing brace — shows the template/builder code since output is dynamic
- **Error handling**: if a registry entry points to a missing file or the pattern is not found, include the entry in the JSON with `"error": "pattern not found"` and `"content": null` rather than failing. The tree browser shows these as broken entries (highlighted in red) so they're visible and fixable

**Metadata enrichment (`tools/prompt-inspector/metadata.py`):**

For each entry:
- **Token count**: `ceil(bytes / 4)` — same heuristic as codebase (`core/src/truncate.rs`)
- **Git last modified**: `git log -1 --format="%ar|%an|%ai" -- <file>` per file. For registry entries pointing into `.rs` files, uses the same per-file git log (not `git log -L`, which is too slow for ~40 entries). Git metadata is collected in parallel (subprocess pool) for performance
- **Source location**: `file:line` for jumping to source

**Output JSON schema:**

```json
{
  "generated_at": "2026-03-18T14:32:00Z",
  "total_tokens": 42350,
  "categories": [
    {
      "name": "System Prompts",
      "total_tokens": 8200,
      "entries": [
        {
          "name": "base instructions",
          "category": "system-prompts",
          "source": "core/prompt.md:1",
          "tokens": 2450,
          "bytes": 9800,
          "git_modified": "3 days ago",
          "git_author": "alice",
          "git_date": "2026-03-15T10:30:00Z",
          "description": "Primary system prompt for the agent",
          "content": "You are Ata, a coding agent...",
          "origin": "auto-discovered"
        }
      ]
    }
  ]
}
```

### 2. Neovim plugin (`tools/prompt-inspector/plugin/`)

A lazy.nvim compatible Lua plugin that provides an interactive interface.

**Structure:**

```
tools/prompt-inspector/plugin/
  lua/
    prompt-inspector/
      init.lua          # Plugin setup, commands, keybindings
      tree.lua          # Tree browser buffer
      preview.lua       # Preview buffer with syntax highlighting
      telescope.lua     # Telescope extension
      data.lua          # Runs generate.py, parses JSON, caches
      context.lua       # Runs dump-initial-context, shows in buffer
```

**Commands:**

| Command | Keybinding | Action |
|---|---|---|
| `:PromptInspector` | `<leader>ip` | Open tree browser + preview |
| `:PromptSearch` | `<leader>is` | Open Telescope fuzzy picker |
| `:PromptContext` | `<leader>if` | Show full assembled startup context |
| `:PromptRefresh` | (in tree) `R` | Force regenerate data |

**Tree browser layout:**

```
┌─ Prompt Inspector ──────────────┬─ Preview ──────────────────────────────┐
│ ▼ System Prompts        8,200t  │ core/templates/compact/prompt.md       │
│   ● base instructions   2,450t  │ Tokens: 245  |  Modified: 2w ago      │
│   ● model instructions    980t  │ By: nima                               │
│   ● review prompt       1,200t  │                                        │
│   ► compact prompt        245t◄ │ You are performing a CONTEXT           │
│ ▼ Tool Descriptions    12,400t  │ CHECKPOINT COMPACTION. Your task is    │
│   ● present_document      890t  │ to create a detailed summary of the    │
│   ● shell tool            340t  │ conversation so far...                 │
│ ► Permissions           1,800t  │                                        │
│ ► Skills                9,200t  │                                        │
│ ► Skill References      3,100t  │                                        │
│ ► Personalities           400t  │                                        │
│ ► Collaboration         2,100t  │                                        │
│ ► Memory System         4,800t  │                                        │
│ ► Research              3,600t  │                                        │
│ ► Coordination          1,200t  │                                        │
│ ► Realtime                300t  │                                        │
│ ► Agent Messages          500t  │                                        │
│ ► Tool Responses        2,400t  │                                        │
│ ► Review                1,600t  │                                        │
│ ► Compact                 350t  │                                        │
└─────────────────────────────────┴────────────────────────────────────────┘
 Total: ~42,350 tokens across 95 entries
 [e] Edit  [s] Sort  [R] Refresh  [q] Quit  [?] Help
```

**Tree buffer keybindings:**

| Key | Action |
|---|---|
| `j/k` | Navigate entries |
| `<CR>` | Expand/collapse category, or open source in split (on entry) |
| `e` | Open source file at line in vertical split |
| `o` | Open source file at line in current window |
| `p` | Toggle preview pinning |
| `s` | Cycle sort: name → tokens (desc) → last modified (desc) |
| `/` | Filter entries by name |
| `R` | Force refresh data |
| `T` | Toggle token count visibility |
| `q` | Close inspector |
| `?` | Show help |

**Preview buffer:**

- Read-only buffer with the prompt content
- Syntax highlighting based on file type (markdown, toml, rust)
- Header line showing: source path, token count, git metadata
- Updates as you navigate the tree
- If you want to edit: `e` opens the actual source file

**Telescope picker (`:PromptSearch`):**

- Fuzzy search across entry names, descriptions, categories, and content
- Preview pane shows full content with metadata header
- `<CR>` opens source file at line
- `<C-v>` opens in vertical split (matching user's existing telescope convention)
- `<C-q>` sends results to quickfix

**Full context view (`:PromptContext`):**

- Runs `cargo run -p codex-cli -- debug dump-initial-context`
- Shows output in a new buffer with markdown syntax highlighting
- Sections are separated and labeled:
  - `## Developer Message` (permissions, developer instructions, memory, coordination, collaboration, realtime, personality, apps, commit attribution)
  - `## Contextual User Message` (user instructions / AGENTS.md, environment context)
  - `## Tools` (all registered tool names + descriptions)
  - `## Base Instructions` (the system prompt / instructions field)
- Token count for the full assembled context shown at top
- Read-only buffer, `q` to close

**Caching:**

- JSON output cached to `/tmp/prompt-inspector-cache.json`
- On `:PromptInspector` / `:PromptSearch`, check cache freshness by computing a hash of: `prompt-registry.toml` content + sorted list of auto-discovered file paths and their mtimes + `generate.py` content. This is more reliable than pure mtime comparison (which can break after `git checkout`)
- If hash differs from cached hash, regenerate automatically
- `R` in tree or `:PromptRefresh` forces regeneration regardless
- The plugin calls `generate.py --metadata-only` first (returns names, categories, tokens, source locations — no content) for fast tree rendering, then lazily fetches full content for the selected entry via `generate.py --entry <name>` when the user navigates to it

**Lazy.nvim spec (added to user's init.lua):**

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

Keybinding group registered with which-key under `<leader>i` = "inspect".

### 3. Rust CLI command (`codex debug dump-initial-context`)

A new subcommand under the existing `debug` command in `cli/src/main.rs`.

**What it does:**

1. Loads config from `~/.ata/config.toml` (respects `--profile`, `-c` overrides)
2. Resolves model, personality, features, sandbox mode, approval policy
3. Assembles the initial context by directly using the same building blocks as `Session::build_initial_context()`:
   - `DeveloperInstructions::from_policy()` for permissions/sandbox
   - Collaboration mode presets for collaboration instructions
   - `get_user_instructions()` for AGENTS.md + skills + JS REPL
   - `EnvironmentContext` for environment XML
   - `ModelInfo::get_model_instructions()` for base instructions
   - `build_specs_with_toolkits()` (or equivalent) for the tool list
4. Prints the fully assembled context to stdout in structured sections
5. Does NOT make any API call or start a session

**Implementation note:** `Session::build_initial_context()` requires a fully initialized `Session` with services, state, and features — too heavy for a dump command. Instead, the dump command directly calls the individual building-block functions (`DeveloperInstructions::from_policy()`, `get_user_instructions()`, `EnvironmentContext::new()`, etc.) with a resolved config. This is slightly more code but avoids initializing the full session machinery. Tool specs are collected separately via the tool registry builder since they are not part of `build_initial_context()` — they go in the API request's `tools` field, not the conversation items.

**Output format:**

```
═══ BASE INSTRUCTIONS (instructions field) ═══════════════════════
Tokens: ~2,450

You are Ata, a coding agent based on GPT-5...
[full base instructions text]

═══ DEVELOPER MESSAGE ════════════════════════════════════════════
Tokens: ~3,200

── Permissions ──
Sandbox mode: workspace-write
[sandbox instructions text]

Approval policy: on-failure
[approval instructions text]

── Developer Instructions (config) ──
[custom developer instructions if any]

── Memory Read Path ──
[memory instructions if memory feature enabled]

── Collaboration Mode ──
Mode: default
[collaboration mode instructions]

── Personality ──
Personality: friendly
[personality spec text]

── Commit Attribution ──
[commit trailer instruction]

═══ CONTEXTUAL USER MESSAGE ══════════════════════════════════════
Tokens: ~8,400

── User Instructions (AGENTS.md) ──
[concatenated AGENTS.md content from hierarchy]

── Skills Section ──
[rendered skills listing]

── Environment Context ──
<environment_context>
  <cwd>/Users/nima/a2a/codex/codex-rs</cwd>
  <shell>zsh</shell>
  ...
</environment_context>

═══ TOOLS (26 registered) ════════════════════════════════════════
Tokens: ~12,000

1. shell — Runs a command in the terminal...
2. apply_patch — Use the apply_patch tool to edit files...
3. present_document — Present a document to the user...
[all tools with descriptions]

═══ TOTAL ════════════════════════════════════════════════════════
Base instructions:     ~2,450 tokens
Developer message:     ~3,200 tokens
User message:          ~8,400 tokens
Tools:                ~12,000 tokens
─────────────────────────────────
Total initial context: ~26,050 tokens
```

**Flags:**

| Flag | Effect |
|---|---|
| `--json` | Output as JSON instead of formatted text |
| `--section <name>` | Show only a specific section (base, developer, user, tools) |
| `--profile <name>` | Use a specific config profile |
| `-c <key=value>` | Config override (same as main CLI) |

**Implementation scope:**

- Add `DumpInitialContext` variant to the debug subcommand enum in `cli/src/main.rs`
- Create `cli/src/dump_context.rs` that:
  - Resolves config, model, features, sandbox policy
  - Calls individual building-block functions to assemble each section
  - Collects tool specs via the tool registry builder
  - Formats and prints the output
- Estimated: ~200-250 lines of Rust, touching `cli/src/main.rs` and a new `cli/src/dump_context.rs`

### 4. Rust annotation convention

All agent-facing inline string content in Rust source files must be annotated with a comment:

```rust
// @agent-facing
const PRESENT_DOCUMENT_TOOL: ToolSpec = ToolSpec {
    description: "Present a document to the user in the reading view...",
    ...
};

// @agent-facing
fn commit_message_trailer_instruction() -> String {
    format!("When you write or edit a git commit message...")
}
```

This annotation:
- Serves as documentation for developers reading the code
- Is machine-readable for validation (`just check-prompts`)
- Does not affect compilation or behavior

**Migration strategy:** Add `@agent-facing` annotations to all ~40 existing locations in a single commit. To minimize upstream merge conflict surface, batch these changes (they're comment-only additions) and keep them small. If an upstream merge later removes or moves an annotated line, `just check-prompts` will catch the drift.

### 5. Validation (`tools/prompt-inspector/validate.py`)

Invoked via `just check-prompts`. Checks three things:

1. **`include_str!` coverage**: grep all `.rs` files for `include_str!` calls, resolve paths, verify each referenced file is known to auto-discovery
2. **`@agent-facing` coverage**: grep all `.rs` files for `// @agent-facing` annotations, verify each has a matching entry in `prompt-registry.toml`
3. **Registry integrity**: for each registry entry, verify the file exists and the pattern is found at the expected location

**Output on failure:**

```
WARN: New include_str! found but not tracked:
  core/src/foo.rs:42 → includes "../../templates/new_prompt.md"

WARN: @agent-facing annotation without registry entry:
  core/src/tools/handlers/bar.rs:150 — pattern: NEW_TOOL_DESCRIPTION

ERROR: Registry entry points to missing pattern:
  entry "Shell tool description" → core/src/tools/spec.rs:create_shell_tool
  Pattern "create_shell_tool" not found (was it renamed?)

2 warnings, 1 error
```

Exit code: 0 if clean, 1 if warnings or errors.

### 6. `just` commands

```just
# Launch prompt inspector in neovim
prompts:
    nvim -c "PromptInspector"

# Run validation
check-prompts:
    python3 tools/prompt-inspector/validate.py

# Dump full assembled context (no neovim needed)
dump-context:
    cargo run -p codex-cli -- debug dump-initial-context
```

Note: the justfile lives at `/Users/nima/a2a/codex/justfile` with `set working-directory := "codex-rs"`, so paths like `tools/prompt-inspector/validate.py` resolve relative to `codex-rs/`.

## AGENTS.md / CLAUDE.md additions

Add to the project's `CLAUDE.md`:

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

## File tree

```
tools/prompt-inspector/
  generate.py               # Discovery + registry + extraction → JSON to stdout
  extractor.py              # Extracts string content from Rust source files
  metadata.py               # Git metadata + token counting
  validate.py               # Drift detection
  prompt-registry.toml      # ~40 entries for inline Rust content
  requirements.txt          # tomli (only needed for Python < 3.11)
  plugin/
    lua/
      prompt-inspector/
        init.lua            # Plugin setup, commands, keybindings, which-key
        tree.lua            # Tree browser buffer rendering + navigation
        preview.lua         # Preview buffer with metadata header
        telescope.lua       # Telescope extension for fuzzy search
        data.lua            # Shells out to generate.py, parses/caches JSON
        context.lua         # Shells out to dump-initial-context, renders buffer
```

## Dependencies

| Component | Dependencies | Notes |
|---|---|---|
| Python backend | Python 3.10+, `tomli` (stdlib in 3.11+) | No pip install needed on 3.11+ |
| Neovim plugin | telescope.nvim (already installed) | Loaded as local plugin via `dir =` |
| Rust command | None new | Uses existing session/config infrastructure |

## Categories

The following categories organize entries in the tree browser:

| Category | Description | Expected entries |
|---|---|---|
| System Prompts | Base instructions, model instructions, review prompt, compact | ~5 |
| Collaboration | Default, plan, execute, pair programming modes, experimental prompt | ~5 |
| Permissions | Sandbox modes, approval policies | ~8 |
| Personalities | Friendly, pragmatic templates | ~2 |
| Skills | SKILL.md files | ~19 |
| Skill References | Reference docs under skills | ~13 |
| Tool Descriptions | All tool description text | ~25 |
| Tool Responses | Response formatting patterns in handlers | ~10 |
| Memory System | Stage 1 system, stage 1 input, consolidation, read path | ~4 |
| Research | Researcher system prompt, prompt builder, zotero instructions | ~3 |
| Coordination | Multi-agent instructions, orchestrator, experimental | ~3 |
| Realtime | Voice mode start/end | ~2 |
| Agent Messages | Spawn messages, worker prompts, role configs | ~5 |
| Review | Review prompts, exit templates, history messages | ~5 |
| Compact | Summarization prompt, summary prefix | ~2 |
| Context Injection | Hierarchical agents, JS REPL, commit attribution, environment | ~4 |

## Non-goals

- Real-time token counting via tiktoken (the 4-bytes heuristic is sufficient for relative comparison)
- Editing prompts from within the inspector (use `e` to jump to source and edit there)
- Tracking MCP tool descriptions from external servers (those are dynamic and third-party)
- CI integration for `check-prompts` (can be added later)
