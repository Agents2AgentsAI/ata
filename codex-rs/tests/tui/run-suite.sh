#!/usr/bin/env bash
#
# Run the agentic behavioral suite over multiple specs, then roll up the
# structured verdicts into tests/tui/STATUS.md.
#
# Runs are SERIAL by design: every run drives a live model (the driver agent
# AND ata's own model) on a shared credential, so parallel runs exhaust quota.
# Each component runs isolated (its own CODEX_HOME + binary copy).
#
# Usage:
#   ./run-suite.sh reading-view goals workspace   # explicit list
#   ./run-suite.sh all                            # every spec (expensive)
#
# Exit code: 0 if every run produced a non-fail verdict, non-zero otherwise.

set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CODEX_RS=$(cd "$SCRIPT_DIR/../.." && pwd)
cd "$CODEX_RS"

if [ $# -eq 0 ]; then
  echo "usage: $0 <component...> | all" >&2
  echo "specs:" >&2
  ls tests/tui/specs/*.md | sed 's#.*/##; s/\.md$//; s/^/  /' | grep -v '^  BACKLOG$' >&2
  exit 2
fi

if [ "$1" = "all" ]; then
  # account-supabase is excluded from `all`: its spec exercises sign-out /
  # revoke, and the harness symlinks the operator's LIVE credential, so an
  # automated run can revoke the shared credential server-side and kill every
  # other run (and the driver). Run it explicitly, with a throwaway account,
  # only when you mean to: `run-suite.sh account-supabase`.
  mapfile -t COMPONENTS < <(ls tests/tui/specs/*.md | sed 's#.*/##; s/\.md$//' \
    | grep -vE '^(BACKLOG|account-supabase)$')
  echo "note: account-supabase excluded from 'all' (credential-destructive; run it explicitly)"
else
  COMPONENTS=("$@")
fi

echo "suite: ${COMPONENTS[*]}"
echo "mode:  serial, isolated (shared-credential quota safety)"
echo ""

declare -A RESULT
fail_count=0
# Per-component wall-clock cap so one hung driver run cannot stall the whole
# serial suite. Override with PER_RUN_TIMEOUT (seconds).
PER_RUN_TIMEOUT="${PER_RUN_TIMEOUT:-2400}"
for comp in "${COMPONENTS[@]}"; do
  echo "========================================================================"
  echo ">>> $comp  (timeout ${PER_RUN_TIMEOUT}s)"
  echo "========================================================================"
  if timeout "$PER_RUN_TIMEOUT" "$SCRIPT_DIR/run-agentic.sh" "$comp" --isolated; then
    RESULT[$comp]="ok"
  else
    code=$?
    # 3 = a real "fail" verdict; 124 = timed out; anything else = inconclusive.
    case "$code" in
      3) RESULT[$comp]="fail" ;;
      124) RESULT[$comp]="timeout" ;;
      *) RESULT[$comp]="error($code)" ;;
    esac
    fail_count=$((fail_count + 1))
  fi
  echo ""
done

echo "========================================================================"
echo ">>> rollup"
echo "========================================================================"
"$SCRIPT_DIR/rollup.sh" || true

echo ""
echo "suite results:"
for comp in "${COMPONENTS[@]}"; do
  printf "  %-22s %s\n" "$comp" "${RESULT[$comp]:-?}"
done

[ "$fail_count" -gt 0 ] && exit 1
exit 0
