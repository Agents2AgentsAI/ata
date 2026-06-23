#!/usr/bin/env bash
#
# Agentic sweep selector.
#
# Instead of a path->spec mapping table, this shows an agent the diff since the
# last sweep and lets it pick which specs to run. Its judgment replaces any
# trigger matrix: it reads intent a glob can't ("mechanical rename, skip" vs
# "this changed the budget logic, run goals"), and a feature added/changed/
# removed shows up in the diff to features/lib.rs or a new handler, so the
# affected spec gets picked with no rule to maintain.
#
# The no-diff drift case (an upstream merge or model swap that no local diff
# points at) is covered by --all: run that on a timer (nightly/weekly) so
# nothing rots when the selector has nothing to react to.
#
# Usage:
#   ./run-due.sh                 # select specs off the diff since last sweep, run them
#   ./run-due.sh --since <ref>   # diff against <ref> instead of the recorded last sweep
#   ./run-due.sh --dry-run       # print the selected specs, do not run
#   ./run-due.sh --all           # run every spec (the timer / drift sweep); ignores the diff
#
# Extra args after the mode pass through to run-agentic.sh (e.g. --isolated).
#
# Cost: the selector is one short agent turn (cheap). Each selected spec is a
# full agentic run (minutes, real tokens) via run-agentic.sh.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CODEX_RS=$(cd "$SCRIPT_DIR/../.." && pwd)
cd "$CODEX_RS"

SPECS_DIR="tests/tui/specs"
LAST_SWEPT_FILE="tests/tui/.last-swept"
CODEX_BIN="${CODEX_BIN:-/home/nima/.local/bin/codex}"

MODE="select"
SINCE=""
PASSTHRU=()
while [ $# -gt 0 ]; do
  case "$1" in
    --all)     MODE="all"; shift ;;
    --dry-run) MODE="dry-run"; shift ;;
    --since)   SINCE="${2:?--since needs a git ref}"; shift 2 ;;
    *)         PASSTHRU+=("$1"); shift ;;
  esac
done

all_specs() { ls "$SPECS_DIR"/*.md 2>/dev/null | xargs -n1 basename | sed 's/\.md$//'; }

run_specs() {
  # $@: spec names. Runs each through run-agentic.sh, continues on failure,
  # and exits non-zero if any selected spec failed (so a timer/CI can gate).
  local failed=0
  for spec in "$@"; do
    echo "=== run-due: $spec ==="
    if ! bash "$SCRIPT_DIR/run-agentic.sh" "$spec" "${PASSTHRU[@]}"; then
      failed=1
    fi
  done
  return $failed
}

if [ "$MODE" = "all" ]; then
  mapfile -t SPECS < <(all_specs)
  echo "run-due: full sweep (${#SPECS[@]} specs)"
  run_specs "${SPECS[@]}"
  RC=$?
  git rev-parse HEAD > "$LAST_SWEPT_FILE"
  exit $RC
fi

# Determine the base ref to diff against.
if [ -z "$SINCE" ]; then
  if [ -f "$LAST_SWEPT_FILE" ]; then
    SINCE=$(cat "$LAST_SWEPT_FILE")
  else
    SINCE="HEAD~1"
    echo "run-due: no $LAST_SWEPT_FILE; defaulting base to HEAD~1" >&2
  fi
fi

if ! git rev-parse --verify --quiet "$SINCE" >/dev/null; then
  echo "run-due: base ref '$SINCE' not found; falling back to HEAD~1" >&2
  SINCE="HEAD~1"
fi

# Scope to this directory tree (CWD is codex-rs/); paths print repo-root-relative.
CHANGED=$(git diff --name-only "$SINCE" HEAD -- . || true)
if [ -z "$CHANGED" ]; then
  echo "run-due: no changes since $SINCE — nothing to select (use --all for a drift sweep)."
  exit 0
fi

SPEC_LIST=$(all_specs | sed 's/^/  - /')

# One cheap agent turn: pick the specs whose behavior the diff could affect.
# It must output ONLY a JSON array of spec base-names on the last line.
SELECT_PROMPT="You are selecting which behavioral specs to re-run after a code \
change. Below are the files changed since the last sweep and the available \
specs (each spec is tests/tui/specs/<name>.md, a capability contract for one \
component). Decide which specs' behavior these changes could plausibly affect \
or invalidate — err toward inclusion when a change touches shared code a spec \
depends on, but skip specs the diff cannot affect (pure docs, unrelated \
components, mechanical renames). A change to a feature flag, a tool handler, or \
a shared boundary should pull in every spec that exercises it. Output ONLY a \
JSON array of spec base-names (e.g. [\"feature-flags\",\"pdf-ingestion\"]) as \
the LAST line of your reply, nothing after it. Empty array if none apply.

Changed files:
$CHANGED

Available specs:
$SPEC_LIST"

echo "run-due: selecting specs for diff $SINCE..HEAD"
SELECT_OUT=$(env -u CODEX_HOME "$CODEX_BIN" exec \
  --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
  -C "$CODEX_RS" "$SELECT_PROMPT" 2>/dev/null || true)

# Extract the last JSON array from the agent output and intersect with real specs.
mapfile -t SELECTED < <(python3 - "$SPECS_DIR" <<PY
import json, re, sys, os, glob
out = """$SELECT_OUT"""
specs = {os.path.basename(p)[:-3] for p in glob.glob(os.path.join(sys.argv[1], "*.md"))}
picked = []
for m in re.findall(r"\[[^\[\]]*\]", out):
    try:
        arr = json.loads(m)
    except Exception:
        continue
    if isinstance(arr, list) and all(isinstance(x, str) for x in arr):
        picked = arr  # keep the last valid array
seen = set()
for name in picked:
    if name in specs and name not in seen:
        seen.add(name)
        print(name)
PY
)

if [ "${#SELECTED[@]}" -eq 0 ]; then
  echo "run-due: selector chose no specs (diff touches nothing a spec covers)."
  [ "$MODE" = "dry-run" ] || git rev-parse HEAD > "$LAST_SWEPT_FILE"
  exit 0
fi

echo "run-due: selected -> ${SELECTED[*]}"
if [ "$MODE" = "dry-run" ]; then
  exit 0
fi

run_specs "${SELECTED[@]}"
RC=$?
git rev-parse HEAD > "$LAST_SWEPT_FILE"
exit $RC
