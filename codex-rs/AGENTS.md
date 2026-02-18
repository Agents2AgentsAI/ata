# Build & test iteration strategy

This workspace has 58 crates and 988 transitive dependencies. Compiling everything from scratch is slow. Follow the rules below to minimize iteration time.

## 1. Always compile-check before running tests

Run `cargo check -p <crate>` first. It is much faster than `cargo test` because it skips codegen and linking. Fix all compile errors before attempting a test run.

## 2. Run the narrowest possible test command

When iterating on a specific test, run **only that test by name** rather than the entire crate's test suite:

```sh
# Best: run a single test by name (fastest iteration loop)
cargo test -p codex-core -- test_name_substring

# Also good: run tests in a single file (for integration test files)
cargo test -p codex-core --test apply_patch_cli -- test_name_substring

# Acceptable: run all tests in a crate (when you changed something broad)
cargo test -p codex-tui
```

Never use `cargo test` (no `-p` flag) during iteration — it compiles and tests the entire workspace.

## 3. Use cargo nextest when running more than a handful of tests

`cargo nextest run` runs tests in parallel processes and is faster than `cargo test` for crates with many tests. Use it when running a full crate suite:

```sh
cargo nextest run -p codex-core
cargo nextest run -p codex-core -- test_name_substring
```

## 4. Understand the crate dependency tiers

Changes in lower tiers force recompilation of all higher tiers. Scope your test runs accordingly:

- **Tier 0 (leaf utilities):** `codex-ansi-escape`, `codex-utils-*`, `codex-apply-patch`, `codex-file-search` — changes here only require testing the crate itself.
- **Tier 1 (protocol/shared):** `codex-protocol`, `codex-common`, `codex-app-server-protocol` — changes here affect `codex-core` and everything above it. Test the crate itself; ask the user before running the full suite.
- **Tier 2 (core):** `codex-core` — this is the bottleneck crate; 20+ crates depend on it. Changes here trigger broad recompilation. Test with `-p codex-core`; ask the user before running the full suite.
- **Tier 3 (leaf consumers):** `codex-tui`, `codex-cli`, `codex-app-server`, `codex-exec`, `codex-mcp-server`, etc. — changes here only require testing the specific crate.

## 5. When to run the full test suite

Only run `just test` when **all** of these are true:
- You changed a Tier 1 crate (`protocol`, `common`, `app-server-protocol`) or Tier 2 (`core`).
- You have already verified your change compiles and passes targeted tests.
- You have asked the user for permission (the full suite takes minutes).

Use `--all-features` only when you changed research-feature-gated code. For research crate changes, use `just test-research`.

For Tier 0 or Tier 3 changes, a full suite run is unnecessary.

## 6. Never build in release mode during iteration

Release builds use `lto = "fat"` and `codegen-units = 1`, which are extremely slow. Never use `--release` or `cargo build --release` unless explicitly building a production binary. All `cargo test`, `cargo check`, and `cargo clippy` commands use the dev/test profile by default — keep it that way.

## 7. Avoid triggering unnecessary recompilation

- Do not run `cargo test --all-features` when your change does not touch feature-gated code. The `--all-features` flag enables optional heavy dependencies (e.g., `research`, `research-repo`) that are not needed for most changes.
- Use `just fix-fast -p <project>` for day-to-day linting. Use `just fix -p <project>` (with `--all-features`) only when you changed research-feature-gated code or for final checks.
- If you only changed test code (not library code), only tests need to recompile — `cargo check -p <crate>` will confirm the library is still fine without recompiling it.

## 8. Crates with especially slow compilation

Be aware of these crates that take disproportionately long to compile, and avoid unnecessary changes to them:
- `codex-app-server-protocol` — 3,400+ lines of protocol types with ~200 derive macro invocations (`Serialize`, `Deserialize`, `JsonSchema`, `TS`). Every type change triggers heavy proc-macro expansion.
- `codex-state` — compiles SQLite from C source via `sqlx`.
- `codex-execpolicy` — depends on `starlark` (a full interpreter).
- `codex-otel` — pulls in the OpenTelemetry stack.
- `codex-tui` — large crate with `ratatui`, `tree-sitter-highlight`, and 234 snapshot files.
