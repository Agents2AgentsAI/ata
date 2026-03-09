# Setting Up LSP & Tree-sitter with Ata

## Overview

Ata integrates **Language Server Protocol (LSP)** and **tree-sitter** to give the AI agent semantic understanding of your codebase — not just text pattern matching.

- **LSP** provides deep, cross-file analysis: go-to-definition, find-references, rename, diagnostics, and more — using the same language servers your editor uses.
- **Tree-sitter** provides fast, local code intelligence: symbol search, call graph analysis, scope-aware grep, and structural exploration — without waiting for a language server to start.

Together, they give Ata two layers of understanding: fast local analysis that's always available, and deep semantic analysis that kicks in when language servers are ready.

## Enabling the Features

LSP and Tree-sitter are experimental features. Enable them in either of two ways:

### Option 1: `/experimental` command (in the TUI)

Run `/experimental` inside Ata and toggle **LSP Integration** and **Tree-sitter Code Intel** on.

### Option 2: `config.toml`

Add to your `config.toml`:

```toml
[features]
lsp = true
treesitter = true
```

## Supported Languages

### LSP — 25 Built-in Language Servers

Language servers are auto-installed on first use to `~/.ata/lsp/`.

| Language | Server | Install Method |
| -------- | ------ | -------------- |
| Rust | rust-analyzer | rustup |
| TypeScript / JavaScript | typescript-language-server | npm |
| Python | pyright | npm |
| Go | gopls | go install |
| C / C++ | clangd | brew / system |
| Swift | sourcekit-lsp | xcrun |
| Java | jdtls | — |
| Kotlin | kotlin-lsp | npm |
| C# | csharp-ls | dotnet tool |
| F# | fsautocomplete | dotnet tool |
| Ruby | rubocop | gem |
| PHP | intelephense | npm |
| Lua | lua-language-server | — |
| Zig | zls | — |
| Nix | nixd | — |
| LaTeX | texlab | — |
| Typst | tinymist | — |
| Bash / Shell | bash-language-server | npm |
| YAML | yaml-language-server | npm |
| Terraform | terraform-ls | npm |
| Dockerfile | dockerfile-language-server | npm |
| Vue | vue-language-server | npm |
| Svelte | svelte-language-server | npm |
| Astro | astro-language-server | npm |
| Clojure | clojure-lsp | — |

### Tree-sitter — 7 Languages with Full Structural Queries

Rust, Python, TypeScript, JavaScript, Go, Java, and Scala.

Other languages are indexed for basic parsing but without semantic queries (callers, symbols, variables).

## Tools Available To Agents

### LSP Tool — 13 Operations

| Operation | Description |
| --------- | ----------- |
| `goToDefinition` | Navigate to a symbol's definition |
| `findReferences` | Find all references to a symbol |
| `hover` | Get type info and documentation |
| `documentSymbol` | List all symbols in a file |
| `workspaceSymbol` | Search symbols across the workspace |
| `goToImplementation` | Find trait/interface implementations |
| `prepareCallHierarchy` | Prepare call hierarchy at a position |
| `incomingCalls` | Find callers of a function |
| `outgoingCalls` | Find functions called by a function |
| `prepareRename` | Validate that a rename is possible |
| `renamePreview` | Preview a rename as a patch |
| `codeActionPreview` | Preview a code action as a patch |
| `diagnostics` | Get errors and warnings |

### Code Intel Tool (Tree-sitter) — 19 Operations

| Operation | Description |
| --------- | ----------- |
| `symbolSearch` | Search symbols by query string |
| `symbols` | List symbols with kind/file filters |
| `callers` | Find call sites for a symbol |
| `tests` | Find test symbols referencing a symbol |
| `variables` | List local variables in a function |
| `implementation` | Return source code for a symbol |
| `structure` | Render project tree for indexed files |
| `peek` | Read a line window from a file |
| `grep` | Regex search across indexed files |
| `chunkIndices` | Compute byte-range chunks for a file |
| `defineSymbol` | Add a human-written definition to a symbol |
| `redefineSymbol` | Update an existing symbol definition |
| `defineFile` | Add a human-written definition to a file |
| `redefineFile` | Update an existing file definition |
| `markFile` | Mark a file as test/docs/config/generated/entryPoint |
| `saveAnnotations` | Persist annotations to disk |
| `loadAnnotations` | Load annotations from disk |
| `addRoot` | Register a new workspace root |
| `removeRoot` | Unregister a workspace root |

## Advanced Configuration

### Disabling a specific language server

```toml
[lsp.rust-analyzer]
disabled = true
```

### Custom language server

```toml
[lsp.pyright]
command = ["/custom/path/to/pyright-langserver", "--stdio"]
extensions = [".py", ".pyi"]
root_markers = ["pyproject.toml", "setup.py"]
```

### Tree-sitter tuning

```toml
[treesitter]
max_file_size = 2097152          # 2 MiB (default: 1 MiB)
ignore_patterns = ["*.min.js", "dist/**"]
ignore_extensions = ["pdf", "whl"]
disabled_languages = ["scala"]
```
