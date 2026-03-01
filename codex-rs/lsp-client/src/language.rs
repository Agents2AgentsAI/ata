//! Extension-to-languageId map for LSP `textDocument/didOpen`.

use std::path::Path;

use phf::phf_map;

/// Returns the LSP `languageId` for a given file path based on its extension.
pub fn language_id_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    EXTENSION_TO_LANGUAGE_ID.get(ext).copied()
}

/// Compile-time perfect-hash map from file extension to LSP languageId.
/// Ported from opencode's language map.
static EXTENSION_TO_LANGUAGE_ID: phf::Map<&'static str, &'static str> = phf_map! {
    // Assembly
    "asm" => "asm",
    "s" => "asm",
    "S" => "asm",
    // C
    "c" => "c",
    "h" => "c",
    // C++
    "cpp" => "cpp",
    "cc" => "cpp",
    "cxx" => "cpp",
    "hpp" => "cpp",
    "hxx" => "cpp",
    "hh" => "cpp",
    // C#
    "cs" => "csharp",
    "csx" => "csharp",
    // Clojure
    "clj" => "clojure",
    "cljs" => "clojurescript",
    "cljc" => "clojure",
    "edn" => "clojure",
    // CoffeeScript
    "coffee" => "coffeescript",
    // Crystal
    "cr" => "crystal",
    // CSS
    "css" => "css",
    // D
    "d" => "d",
    // Dart
    "dart" => "dart",
    // Dockerfile
    "dockerfile" => "dockerfile",
    // Elixir
    "ex" => "elixir",
    "exs" => "elixir",
    // Elm
    "elm" => "elm",
    // Erlang
    "erl" => "erlang",
    "hrl" => "erlang",
    // F#
    "fs" => "fsharp",
    "fsi" => "fsharp",
    "fsx" => "fsharp",
    // Gleam
    "gleam" => "gleam",
    // Go
    "go" => "go",
    // Groovy
    "groovy" => "groovy",
    "gvy" => "groovy",
    // Haskell
    "hs" => "haskell",
    "lhs" => "haskell",
    // HTML
    "html" => "html",
    "htm" => "html",
    // Java
    "java" => "java",
    // JavaScript
    "js" => "javascript",
    "mjs" => "javascript",
    "cjs" => "javascript",
    // JSX
    "jsx" => "javascriptreact",
    // JSON
    "json" => "json",
    "jsonc" => "jsonc",
    // Julia
    "jl" => "julia",
    // Kotlin
    "kt" => "kotlin",
    "kts" => "kotlin",
    // LaTeX
    "tex" => "latex",
    "sty" => "latex",
    "cls" => "latex",
    // Less
    "less" => "less",
    // Lua
    "lua" => "lua",
    // Makefile
    "mk" => "makefile",
    // Markdown
    "md" => "markdown",
    "markdown" => "markdown",
    // Nim
    "nim" => "nim",
    // Nix
    "nix" => "nix",
    // Objective-C
    "m" => "objective-c",
    "mm" => "objective-cpp",
    // OCaml
    "ml" => "ocaml",
    "mli" => "ocaml",
    // Pascal
    "pas" => "pascal",
    // Perl
    "pl" => "perl",
    "pm" => "perl",
    // PHP
    "php" => "php",
    // PowerShell
    "ps1" => "powershell",
    "psm1" => "powershell",
    "psd1" => "powershell",
    // Prisma
    "prisma" => "prisma",
    // Proto
    "proto" => "proto",
    // Python
    "py" => "python",
    "pyi" => "python",
    "pyw" => "python",
    // R
    "r" => "r",
    "R" => "r",
    // Ruby
    "rb" => "ruby",
    "erb" => "ruby",
    // Rust
    "rs" => "rust",
    // SASS/SCSS
    "sass" => "sass",
    "scss" => "scss",
    // Scala
    "scala" => "scala",
    "sc" => "scala",
    // Shell
    "sh" => "shellscript",
    "bash" => "shellscript",
    "zsh" => "shellscript",
    "fish" => "shellscript",
    // SQL
    "sql" => "sql",
    // Svelte
    "svelte" => "svelte",
    // Swift
    "swift" => "swift",
    // Terraform
    "tf" => "terraform",
    "tfvars" => "terraform",
    // TOML
    "toml" => "toml",
    // TypeScript
    "ts" => "typescript",
    "mts" => "typescript",
    "cts" => "typescript",
    // TSX
    "tsx" => "typescriptreact",
    // Verilog
    "v" => "verilog",
    "sv" => "systemverilog",
    // VHDL
    "vhd" => "vhdl",
    "vhdl" => "vhdl",
    // Vue
    "vue" => "vue",
    // XML
    "xml" => "xml",
    "xsl" => "xml",
    "xslt" => "xml",
    // YAML
    "yaml" => "yaml",
    "yml" => "yaml",
    // Zig
    "zig" => "zig",
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn known_extensions() {
        assert_eq!(language_id_for_path(Path::new("main.rs")), Some("rust"));
        assert_eq!(language_id_for_path(Path::new("app.tsx")), Some("typescriptreact"));
        assert_eq!(language_id_for_path(Path::new("lib.py")), Some("python"));
        assert_eq!(language_id_for_path(Path::new("main.go")), Some("go"));
        assert_eq!(language_id_for_path(Path::new("file.cpp")), Some("cpp"));
        assert_eq!(language_id_for_path(Path::new("page.swift")), Some("swift"));
    }

    #[test]
    fn unknown_extension() {
        assert_eq!(language_id_for_path(Path::new("file.xyz")), None);
    }

    #[test]
    fn no_extension() {
        assert_eq!(language_id_for_path(&PathBuf::from("Makefile")), None);
    }
}
