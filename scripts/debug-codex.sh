#!/bin/bash

# Set "chatgpt.cliExecutable": "/Users/<USERNAME>/code/codex/scripts/debug-codex.sh" in VSCode settings to always get the 
# latest codex-rs binary when debugging Codex Extension.


set -euo pipefail

ATA_RS_DIR=$(realpath "$(dirname "$0")/../codex-rs")
(cd "$ATA_RS_DIR" && cargo run --quiet --bin ata -- "$@")