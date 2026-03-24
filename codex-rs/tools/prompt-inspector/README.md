# Prompt Inspector

Browse all agent-facing content (prompts, instructions, tool descriptions, skills) with token counts, git metadata, and jump-to-source — from inside neovim.

## Setup

Add to your `init.lua` (lazy.nvim):

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

Requires: Python 3.10+, Telescope.

## Usage

| Key | Command | What it does |
|-----|---------|-------------|
| `<leader>ip` | `:PromptInspector` | Toggle sidebar (tree browser) |
| `<leader>is` | `:PromptSearch` | Fuzzy search all prompts (Telescope) |
| `<leader>if` | `:PromptContext` | Dump full assembled agent startup context |

### Sidebar keybindings

| Key | Action |
|-----|--------|
| `<CR>` | Toggle category / open source file |
| `<BS>` | Collapse parent category |
| `<C-v>` | Open source file in vertical split |
| `s` | Cycle sort: name / tokens / date |
| `/` | Filter entries (auto-expands matches) |
| `R` | Refresh data |
| `T` | Toggle token counts |
| `q` | Close sidebar |
| `?` | Help |

## Just commands

```
just prompts        # Open inspector in a fresh nvim
just check-prompts  # Validate registry integrity
just dump-context   # Dump assembled agent context to terminal
```

## Adding new agent-facing content

- **Template files** (`.md`/`.txt` in `core/templates/`, `protocol/src/prompts/`, etc.): auto-discovered.
- **Inline Rust strings** (tool descriptions, prompt constants): add `// @agent-facing` above the const/function, add an entry to `prompt-registry.toml`.
- **Skill files**: place `SKILL.md` under `skills/src/assets/`. Auto-discovered.

Run `just check-prompts` to verify.
