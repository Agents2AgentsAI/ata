#!/bin/sh
# Pretty-print the latest rollout file in a readable format.
# Usage:
#   ./debug-rollout.sh          show entire latest session
#   ./debug-rollout.sh 20       show last 20 entries
#   ./debug-rollout.sh -f       tail -f (live follow)
#   ./debug-rollout.sh -f 20    show last 20 then follow

LATEST=$(ls -t ~/.ata/sessions/2026/*/*/*.jsonl 2>/dev/null | head -1)

if [ -z "$LATEST" ]; then
  echo "No rollout files found"
  exit 1
fi

echo "=== $LATEST ==="
echo ""

FOLLOW=false
BACKFILL=""

for arg in "$@"; do
  case "$arg" in
    -f) FOLLOW=true ;;
    *[!0-9]*) echo "Unknown argument: $arg"; exit 1 ;;
    *)  BACKFILL="$arg" ;;
  esac
done

PYFILE=$(mktemp /tmp/debug-rollout.XXXXXX.py)
trap 'rm -f "$PYFILE"' EXIT

cat > "$PYFILE" << 'PYEOF'
import json, sys

sys.stdout.reconfigure(line_buffering=True)

for line in sys.stdin:
    try:
        obj = json.loads(line)
    except (json.JSONDecodeError, ValueError):
        continue
    t = obj.get("type", "")

    if t == "response_item":
        p = obj["payload"]
        kind = p.get("type", "")

        if kind == "reasoning":
            for s in p.get("summary", []):
                txt = s.get("text", "")
                print(f"\033[33mTHINKING:\033[0m {txt}")
            print()

        elif kind == "function_call":
            name = p.get("name", "?")
            args = p.get("arguments", "")
            if len(args) > 300:
                args = args[:300] + "..."
            print(f"\033[36mTOOL CALL:\033[0m {name}({args})")
            print()

        elif kind == "function_call_output":
            out = p.get("output", "")
            if len(out) > 500:
                out = out[:500] + "..."
            print(f"\033[35mTOOL RESULT:\033[0m {out}")
            print()

        elif kind == "message":
            role = p.get("role", "?").upper()
            for c in p.get("content", []):
                ctype = c.get("type", "")
                if ctype == "input_text":
                    txt = c["text"]
                    if role == "DEVELOPER" and len(txt) > 200:
                        txt = txt[:200] + "..."
                    if role == "USER":
                        print(f"\033[32m{role}:\033[0m {txt}")
                    else:
                        print(f"\033[34m{role}:\033[0m {txt}")
                elif ctype == "output_text":
                    txt = c.get("text", "")
                    print(f"\033[32mAGENT:\033[0m {txt}")
            print()

    elif t == "event_msg":
        p = obj.get("payload", {})
        etype = p.get("type", "")
        if etype == "agent_reasoning":
            txt = p.get("text", "")
            if txt.strip():
                print(f"\033[33m  ...thinking: {txt[:200]}\033[0m")
PYEOF

if $FOLLOW; then
  if [ -n "$BACKFILL" ]; then
    tail -n "$BACKFILL" -f "$LATEST" | python3 "$PYFILE"
  else
    tail -n 0 -f "$LATEST" | python3 "$PYFILE"
  fi
elif [ -n "$BACKFILL" ]; then
  tail -n "$BACKFILL" "$LATEST" | python3 "$PYFILE"
else
  cat "$LATEST" | python3 "$PYFILE"
fi
