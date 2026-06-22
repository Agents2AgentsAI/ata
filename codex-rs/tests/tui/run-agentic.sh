#!/usr/bin/env bash
#
# Agentic behavioral test runner.
#
# Launches Claude Code headlessly on a component spec. The agent reads
# the spec, drives the real ata binary (CLI + tmux TUI + in-app model),
# and writes a report to tests/tui/reports/.
#
# Usage:
#   ./run-agentic.sh                 # default component: workspace
#   ./run-agentic.sh workspace       # explicit component
#   ./run-agentic.sh workspace --model opus   # extra args pass through to claude
#   ./run-agentic.sh research --isolated --service-tier priority
#                                    # exercise the tier-gated spawn/model paths
#
# Cost: real model tokens (the Claude session AND ata's own model for
# skill-layer probes). Minutes, not seconds. Not meant for every push.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CODEX_RS=$(cd "$SCRIPT_DIR/../.." && pwd)
cd "$CODEX_RS"

COMPONENT="${1:-workspace}"
[ $# -gt 0 ] && shift

# --isolated: run against a private CODEX_HOME and a private copy of the
# binary, so several component runs can execute in parallel without sharing
# sessions, workspaces, skills, or the binary under test.
ISOLATED=0
if [ "${1:-}" = "--isolated" ]; then
  ISOLATED=1
  shift
fi

# --service-tier <tier>: set `service_tier` in the isolated config so code
# paths that only run under an active service tier (priority, flex, …) are
# exercised — e.g. the sub-agent spawn validation that resolves the child
# model only when a tier is set. OFF by default (the suite holds this axis
# constant), so opt in to cover it. Use a tier your test account/model
# actually supports, or unrelated capabilities will fail on tier rejection.
# Only meaningful with --isolated (the driver's own config is untouched).
SERVICE_TIER=""
if [ "${1:-}" = "--service-tier" ]; then
  SERVICE_TIER="${2:-}"
  shift 2
fi

SPEC="tests/tui/specs/$COMPONENT.md"
if [ ! -f "$SPEC" ]; then
  echo "error: no spec for component '$COMPONENT'" >&2
  echo "available specs:" >&2
  ls tests/tui/specs/ | sed 's/\.md$//; s/^/  /' >&2
  exit 2
fi

DATE=$(date -u +%Y-%m-%d)

CODEX_BIN="${CODEX_BIN:-/home/nima/.local/bin/codex}"
if ! command -v "$CODEX_BIN" >/dev/null 2>&1 && [ ! -x "$CODEX_BIN" ]; then
  echo "error: driver codex CLI not found (set CODEX_BIN; tried '$CODEX_BIN')" >&2
  exit 2
fi
if ! command -v tmux >/dev/null 2>&1; then
  echo "error: tmux is required (the agent drives the TUI through it)" >&2
  exit 2
fi

if [ ! -x target/debug/ata ]; then
  echo "building ata (debug)..."
  cargo build -p codex-cli --bin ata
fi

VERDICT_JSON="tests/tui/reports/$DATE-$COMPONENT.verdict.json"
PROMPT="Run the behavioral spec at tests/tui/specs/$COMPONENT.md against \
./target/debug/ata and write the report. Follow the protocol in \
tests/tui/README.md exactly: exercise every capability in the spec, then \
probe adversarially beyond it; judge agent-layer behavior from the session \
JSONL, not the rendered pane; never modify product code; clean up everything \
you create; write the report to tests/tui/reports/ named by ISO date and \
component. When you report an issue, frame the RIGHT fix in terms of the \
engineering principles in tests/tui/ENGINEERING-PRINCIPLES.md (fix at the \
choke point, prefer native capability, design the failure out, centralize \
boundary transforms, fail fast and typed, root-cause before patching) so \
whoever fixes it follows them. \
\
FALLBACKS ARE FAILURES (judge this on EVERY capability): a capability the spec \
describes must actually work through its intended path. If that path errors, \
returns nothing, times out, or is unavailable, and the agent only reaches a \
good-looking result by working around it (shelling out, using a different \
tool, doing the work by hand, reading sources directly, retrying onto another \
path), that is a real defect — file it as a finding, default HIGH severity, \
EVEN WHEN the final answer is correct. A feature that 'works' only via a \
workaround is broken. Do not be satisfied by the end result: read the session \
JSONL for failures the agent silently recovered from — tool or sub-agent \
spawn errors, 'could not ...' / 'failed to ...' messages, non-zero exits, a \
retry that switches strategy — and file each one, naming the capability that \
did not work. A correct outcome never excuses a recovered failure, and a run \
with one MUST NOT reach \"pass\". Distinguish this from the agent freely \
CHOOSING a valid path the spec permits (fine); the defect is the intended \
path FAILING and being silently substituted. \
\
STRUCTURED VERDICT (required, in addition to the prose report): also write a \
machine-readable verdict to $VERDICT_JSON. It MUST be valid JSON with exactly \
these keys: component (string), spec (string path), report (string path to \
your prose report), verdict (one of \"pass\", \"partial\", \"fail\"), \
timestamp_utc (ISO-8601), binary_version (string from ata --version), \
capabilities_total (int: how many distinct spec capabilities you judged), \
capabilities_passed (int), findings (array; each item has: id (string), \
severity (one of \"high\", \"medium\", \"low\"), capability (string naming the \
spec section), title (string), status (\"open\")), notes (one-line string). \
Verdict rules, apply them exactly: \"fail\" if ANY finding is high severity \
(a real defect that violates the spec); \"partial\" if there are only \
medium/low findings OR you could not exercise some capabilities; \"pass\" only \
if findings is empty AND you exercised every capability in the spec. Do not \
soften a real defect to reach pass. Write the JSON file as the LAST thing you \
do, after cleanup, so it reflects the final judgment."

echo "component: $COMPONENT"
echo "spec:      $SPEC"
echo "binary:    $(./target/debug/ata --version 2>&1 | head -1)"
echo ""

# The binary under test must shadow any PATH-installed ata: the in-app agent
# runs `ata` from PATH (a prior run tested the installed binary by mistake).
export PATH="$CODEX_RS/target/debug:$PATH"

if [ "$ISOLATED" = "1" ]; then
  ISO_HOME="/tmp/ata-agentic-$COMPONENT"
  rm -rf "$ISO_HOME"
  mkdir -p "$ISO_HOME/bin"
  cp target/debug/ata "$ISO_HOME/bin/ata"
  # Symlink to a LIVE auth file (not a frozen copy): ChatGPT OAuth tokens
  # refresh/expire, so a snapshot copied at launch goes stale during long
  # runs and the in-app model's calls start failing on revoked auth. Prefer
  # the driver's own ~/.codex/auth.json (kept fresh by the running codex
  # driver), falling back to ~/.ata.
  #
  # EXCEPTION: the account-supabase spec exercises sign-out / logout / revoke.
  # A symlink would point those at the operator's real credential and revoke
  # it server-side (killing the driver and every other run). Use a detached
  # COPY for that component so a local logout deletes the copy, not the live
  # file. (Server-side revocation of the real token is still possible; the
  # spec's CREDENTIAL SAFETY note forbids it.)
  AUTH_LINK_CMD="ln -sf"
  [ "$COMPONENT" = "account-supabase" ] && AUTH_LINK_CMD="cp"
  if [ -f "$HOME/.codex/auth.json" ]; then
    $AUTH_LINK_CMD "$HOME/.codex/auth.json" "$ISO_HOME/auth.json"
  elif [ -f "$HOME/.ata/auth.json" ]; then
    $AUTH_LINK_CMD "$HOME/.ata/auth.json" "$ISO_HOME/auth.json"
  fi
  SERVICE_TIER_TOML=""
  [ -n "$SERVICE_TIER" ] && SERVICE_TIER_TOML="service_tier = \"$SERVICE_TIER\""
  cat > "$ISO_HOME/config.toml" <<EOF
$SERVICE_TIER_TOML
[projects."$(dirname "$CODEX_RS")"]
trust_level = "trusted"

[projects."$CODEX_RS"]
trust_level = "trusted"
EOF
  [ -n "$SERVICE_TIER" ] && echo "service_tier: $SERVICE_TIER (isolated config)"
  # IMPORTANT: do NOT export CODEX_HOME. The codex DRIVER and the ata binary
  # under test both read CODEX_HOME; sharing it would mix the driver's own
  # session JSONL into the test target's home and break JSONL judging. The
  # driver keeps its own ~/.codex; the test target gets $ISO_HOME inline on
  # EVERY invocation (tmux launches AND plain CLI calls).
  PROMPT="$PROMPT ISOLATION NOTES: this run is isolated. The ata binary under \
test is $ISO_HOME/bin/ata and its home is CODEX_HOME=$ISO_HOME (sessions \
JSONLs, workspaces, skills, caches live there, NOT ~/.ata and NOT your own \
~/.codex). You MUST set CODEX_HOME=$ISO_HOME inline on EVERY ata invocation — \
both CLI calls, e.g. \`CODEX_HOME=$ISO_HOME $ISO_HOME/bin/ata workspace list\`, \
and tmux launches, e.g. \`tmux -L ata-$COMPONENT new-session -d -s probe \
\"CODEX_HOME=$ISO_HOME $ISO_HOME/bin/ata --yolo\"\`. Never run the bare ata \
binary without that prefix (it would hit the wrong home). Use the -L \
ata-$COMPONENT tmux socket for every tmux command so parallel runs never share \
a tmux server. To read the test target's session JSONL, look under \
$ISO_HOME/sessions, never ~/.codex/sessions (that is your own driver log). \
Other agentic runs may be executing concurrently; ignore their processes and \
never touch ~/.ata, ~/.codex, or other /tmp/ata-agentic-* directories. Do NOT \
run any git state-changing command (stash/reset/checkout/clean/restore)."
  echo "isolated:  test CODEX_HOME=$ISO_HOME (driver uses its own ~/.codex)"
fi

if [ -n "$SERVICE_TIER" ]; then
  PROMPT="$PROMPT SERVICE TIER ACTIVE: this run sets service_tier=\"$SERVICE_TIER\" \
in the target config, so sub-agent spawns and model resolution take the \
tier-gated code path. Deliberately exercise a capability that spawns a \
sub-agent (e.g. paper synthesis) and confirm the spawn SUCCEEDS under the \
tier. A spawn or model-resolution error under the active tier — 'could not \
resolve the child model', a failed delegate, or a silent fallback to doing \
the work in the main session — is a high-severity finding even if the end \
result is correct."
fi

# Driver: system codex exec (its own quota + ~/.codex), not claude. It reads
# the spec, drives the ata-under-test, and writes the report.
CODEX_BIN="${CODEX_BIN:-/home/nima/.local/bin/codex}"
env -u CODEX_HOME "$CODEX_BIN" exec \
  --dangerously-bypass-approvals-and-sandbox --skip-git-repo-check \
  -C "$CODEX_RS" "$PROMPT" "$@"

REPORT=$(ls -t tests/tui/reports/*"$COMPONENT"*.md 2>/dev/null | grep -v '\.verdict\.json$' | head -1)
echo ""
if [ -n "$REPORT" ]; then
  echo "report: $REPORT"
  echo "----"
  sed -n '/^## Issues/,/^## Behaviors/p' "$REPORT" | head -40
else
  echo "warning: no report found for '$COMPONENT' — check the agent output above" >&2
fi

# Structured verdict: the parseable PASS/FAIL gate. Find the freshest verdict
# file for this component (the agent writes $DATE-$COMPONENT.verdict.json).
VERDICT=$(ls -t tests/tui/reports/*"$COMPONENT"*.verdict.json 2>/dev/null | head -1)
echo ""
if [ -z "$VERDICT" ]; then
  echo "error: no structured verdict written for '$COMPONENT' — agentic run is" >&2
  echo "       inconclusive (the agent must write *.verdict.json)." >&2
  exit 1
fi
if ! python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$VERDICT" 2>/dev/null; then
  echo "error: verdict file is not valid JSON: $VERDICT" >&2
  exit 1
fi
RESULT=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1])).get('verdict','?'))" "$VERDICT")
echo "verdict file: $VERDICT"
python3 - "$VERDICT" <<'PY'
import json, sys
v = json.load(open(sys.argv[1]))
f = v.get("findings", []) or []
hi = sum(1 for x in f if x.get("severity") == "high")
md = sum(1 for x in f if x.get("severity") == "medium")
lo = sum(1 for x in f if x.get("severity") == "low")
print(f"VERDICT: {str(v.get('verdict','?')).upper()}  "
      f"(caps {v.get('capabilities_passed','?')}/{v.get('capabilities_total','?')}, "
      f"findings: {hi} high / {md} med / {lo} low)")
if v.get("notes"):
    print(f"notes: {v['notes']}")
PY
# Exit non-zero on a real failure so callers (run-suite.sh, CI) can gate.
[ "$RESULT" = "fail" ] && exit 3
exit 0
