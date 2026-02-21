## Installing & building

### System requirements

| Requirement                 | Details                                                         |
| --------------------------- | --------------------------------------------------------------- |
| Operating systems           | macOS 12+, Ubuntu 20.04+/Debian 10+, or Windows 11 **via WSL2** |
| Git (optional, recommended) | 2.23+ for built-in PR helpers                                   |
| RAM                         | 4-GB minimum (8-GB recommended)                                 |

### DotSlash

The GitHub Release also contains a [DotSlash](https://dotslash-cli.com/) file for the Ata CLI named `ata`. Using a DotSlash file makes it possible to make a lightweight commit to source control to ensure all contributors use the same version of an executable, regardless of what platform they use for development.

### Install via curl (macOS/Linux)

You can install the latest release with a `curl | sh` one-liner:

```bash
curl -fsSL https://agents2agents.ai/ata/install.sh | sh
```

Options (pass with `sh -s -- ...`):

```bash
# Install into a custom directory
curl -fsSL https://agents2agents.ai/ata/install.sh | sh -s -- --bin-dir "$HOME/bin"

# Install a specific release tag
curl -fsSL https://agents2agents.ai/ata/install.sh | sh -s -- --version 1.2.3
```

### Build from source

```bash
# Clone the repository and navigate to the root of the Cargo workspace.
git clone https://github.com/Agents2AgentsAI/ata.git
cd ata/codex-rs

# Install the Rust toolchain, if necessary.
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt
rustup component add clippy
# Install helper tools used by the workspace justfile:
cargo install just
# Optional: install nextest for the `just test` helper
cargo install --locked cargo-nextest

# Build Codex.
cargo build

# Launch the TUI with a sample prompt.
cargo run --bin ata -- "explain this codebase to me"

# After making changes, use the root justfile helpers (they default to codex-rs):
just fmt
just fix -p <crate-you-touched>

# Run the relevant tests (project-specific is fastest), for example:
cargo test -p codex-tui
# If you have cargo-nextest installed, `just test` runs the test suite via nextest:
just test
# If you specifically want the full `--all-features` matrix, use:
cargo test --all-features
```

## Tracing / verbose logging

Ata is build on a fork of Codex, which is is written in Rust, so it honors the `RUST_LOG` environment variable to configure its logging behavior.

The TUI defaults to `RUST_LOG=codex_core=info,codex_tui=info,codex_rmcp_client=info` and log messages are written to `~/.ata/log/codex-tui.log` by default. For a single run, you can override the log directory with `-c log_dir=...` (for example, `-c log_dir=./.codex-log`).

```bash
tail -F ~/.ata/log/codex-tui.log
```

By comparison, the non-interactive mode (`ata exec`) defaults to `RUST_LOG=error`, but messages are printed inline, so there is no need to monitor a separate file.

See the Rust documentation on [`RUST_LOG`](https://docs.rs/env_logger/latest/env_logger/#enabling-logging) for more information on the configuration options.
