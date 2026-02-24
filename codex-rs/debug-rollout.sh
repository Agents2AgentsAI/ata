#!/bin/sh
# Pretty-print the latest rollout file in a readable format.
# Includes subagent logs by default (discovered via parent-child relationship).
# Usage:
#   ./debug-rollout.sh              show entire latest session (with subagents)
#   ./debug-rollout.sh 20           show last 20 entries
#   ./debug-rollout.sh -f           tail -f (live follow)
#   ./debug-rollout.sh -f 20        show last 20 then follow
#   ./debug-rollout.sh --no-sub     show only main agent (no subagents)

LATEST=$(ls -t ~/.ata/sessions/*/*/*/*.jsonl 2>/dev/null | head -1)

if [ -z "$LATEST" ]; then
  echo "No rollout files found"
  exit 1
fi

echo "=== $LATEST ==="
echo ""

FOLLOW=false
BACKFILL=""
NO_SUB=false

for arg in "$@"; do
  case "$arg" in
    -f) FOLLOW=true ;;
    --no-sub) NO_SUB=true ;;
    *[!0-9]*) echo "Unknown argument: $arg"; exit 1 ;;
    *)  BACKFILL="$arg" ;;
  esac
done

# Derive the sessions root directory (4 levels above the rollout file).
SESSIONS_DIR=$(dirname "$(dirname "$(dirname "$(dirname "$LATEST")")")")

PYFILE=$(mktemp /tmp/debug-rollout.XXXXXX.py)
trap 'rm -f "$PYFILE"' EXIT

cat > "$PYFILE" << 'PYEOF'
import json, sys, os, time, signal
from pathlib import Path

# Let the process die quietly on broken pipe (e.g. piped to head).
signal.signal(signal.SIGPIPE, signal.SIG_DFL)

# ── Colors ──────────────────────────────────────────────────────────
C_RESET  = "\033[0m"
C_BOLD   = "\033[1m"
C_DIM    = "\033[2m"
C_YELLOW = "\033[33m"
C_CYAN   = "\033[36m"
C_MAGENTA= "\033[35m"
C_GREEN  = "\033[32m"
C_BLUE   = "\033[34m"

sys.stdout.reconfigure(line_buffering=True)

# ── Session discovery ───────────────────────────────────────────────
def get_session_meta(filepath):
    """Read first line and return (thread_id, parent_thread_id | None)."""
    try:
        with open(filepath) as f:
            line = f.readline()
            if not line:
                return None
            obj = json.loads(line)
            if obj.get("type") != "session_meta":
                return None
            payload = obj.get("payload", {})
            thread_id = payload.get("id")
            source = payload.get("source", "")
            parent_id = None
            if isinstance(source, dict):
                sub = source.get("subagent", {})
                if isinstance(sub, dict):
                    for val in sub.values():
                        if isinstance(val, dict) and "parent_thread_id" in val:
                            parent_id = val["parent_thread_id"]
            return (thread_id, parent_id)
    except Exception:
        return None


def discover_subagent_files(main_file, sessions_dir):
    """Walk the sessions dir and find all descendant rollout files."""
    main_meta = get_session_meta(main_file)
    if not main_meta:
        return []
    main_id = main_meta[0]
    known_ids = {main_id}

    # Build map: thread_id -> (filepath, parent_id) for every file
    all_files = sorted(Path(sessions_dir).glob("*/*/*/*.jsonl"))
    file_map = {}
    for f in all_files:
        fpath = str(f)
        if fpath == main_file:
            continue
        meta = get_session_meta(fpath)
        if meta and meta[0]:
            file_map[meta[0]] = (fpath, meta[1])

    # Iteratively find transitive descendants
    result = []
    changed = True
    while changed:
        changed = False
        for tid, (fpath, parent_id) in list(file_map.items()):
            if parent_id in known_ids and tid not in known_ids:
                known_ids.add(tid)
                result.append((fpath, tid))
                changed = True
    return result


def short_id(thread_id):
    return thread_id[:8] if thread_id else "?"

# ── Event formatting ────────────────────────────────────────────────
def format_event(obj, label=None):
    """Format a single JSONL event. Returns a string or None."""
    prefix = ""
    if label:
        prefix = f"{C_DIM}[{label}]{C_RESET} "

    t = obj.get("type", "")

    if t == "response_item":
        p = obj.get("payload", {})
        kind = p.get("type", "")

        if kind == "reasoning":
            lines = []
            for s in p.get("summary", []):
                txt = s.get("text", "")
                lines.append(f"{prefix}{C_YELLOW}THINKING:{C_RESET} {txt}")
            return "\n".join(lines) + "\n" if lines else None

        elif kind == "function_call":
            name = p.get("name", "?")
            args = p.get("arguments", "")
            if len(args) > 300:
                args = args[:300] + "..."
            return f"{prefix}{C_CYAN}TOOL CALL:{C_RESET} {name}({args})\n"

        elif kind == "function_call_output":
            out = p.get("output", "")
            if len(out) > 500:
                out = out[:500] + "..."
            return f"{prefix}{C_MAGENTA}TOOL RESULT:{C_RESET} {out}\n"

        elif kind == "message":
            role = p.get("role", "?").upper()
            lines = []
            for c in p.get("content", []):
                ctype = c.get("type", "")
                if ctype == "input_text":
                    txt = c.get("text", "")
                    if role == "DEVELOPER" and len(txt) > 200:
                        txt = txt[:200] + "..."
                    color = C_GREEN if role == "USER" else C_BLUE
                    lines.append(f"{prefix}{color}{role}:{C_RESET} {txt}")
                elif ctype == "output_text":
                    txt = c.get("text", "")
                    lines.append(f"{prefix}{C_GREEN}AGENT:{C_RESET} {txt}")
            return "\n".join(lines) + "\n" if lines else None

    elif t == "event_msg":
        p = obj.get("payload", {})
        etype = p.get("type", "")
        if etype == "agent_reasoning":
            txt = p.get("text", "")
            if txt.strip():
                return f"{prefix}{C_YELLOW}  ...thinking: {txt[:200]}{C_RESET}"

    return None

# ── Read helpers ────────────────────────────────────────────────────
def read_all_events(filepath, label):
    """Read every line from a rollout file, tagging each with a label."""
    events = []
    try:
        with open(filepath) as f:
            for line in f:
                try:
                    obj = json.loads(line)
                    obj["_label"] = label
                    events.append(obj)
                except (json.JSONDecodeError, ValueError):
                    continue
    except OSError:
        pass
    return events

# ── Static (non-follow) mode ───────────────────────────────────────
def run_static(main_file, sub_files, backfill):
    main_meta = get_session_meta(main_file)
    has_subs = len(sub_files) > 0
    main_label = f"main:{short_id(main_meta[0])}" if has_subs and main_meta else None

    all_events = read_all_events(main_file, main_label)
    for fpath, tid in sub_files:
        all_events.extend(read_all_events(fpath, f"sub:{short_id(tid)}"))

    # Stable sort by timestamp (preserves intra-file order for equal timestamps)
    all_events.sort(key=lambda e: e.get("timestamp", ""))

    if backfill:
        # Keep only the last N displayable events (plus everything after them)
        displayable = [e for e in all_events
                       if e.get("type") in ("response_item", "event_msg")]
        if len(displayable) > backfill:
            cutoff_ts = displayable[-backfill].get("timestamp", "")
            all_events = [e for e in all_events
                          if e.get("timestamp", "") >= cutoff_ts]

    for ev in all_events:
        label = ev.pop("_label", None)
        output = format_event(ev, label)
        if output:
            print(output)

# ── Follow mode ─────────────────────────────────────────────────────
def run_follow(main_file, sessions_dir, sub_files, backfill, no_sub):
    main_meta = get_session_meta(main_file)
    main_id = main_meta[0] if main_meta else None
    has_subs = len(sub_files) > 0 or not no_sub
    known_ids = {main_id} if main_id else set()

    # {filepath: (file_handle, label)}
    open_files = {}

    def open_and_register(fpath, label, tid):
        fh = open(fpath)
        if backfill:
            lines = fh.readlines()
            start = max(0, len(lines) - backfill)
            for line in lines[start:]:
                try:
                    obj = json.loads(line)
                    output = format_event(obj, label)
                    if output:
                        print(output)
                except Exception:
                    pass
        else:
            fh.seek(0, 2)  # seek to end
        open_files[fpath] = (fh, label)
        if tid:
            known_ids.add(tid)

    main_label = f"main:{short_id(main_id)}" if has_subs and main_id else None
    open_and_register(main_file, main_label, main_id)
    for fpath, tid in sub_files:
        open_and_register(fpath, f"sub:{short_id(tid)}", tid)

    scan_counter = 0
    try:
        while True:
            got_data = False
            for fpath, (fh, label) in list(open_files.items()):
                line = fh.readline()
                while line:
                    got_data = True
                    try:
                        obj = json.loads(line)
                        output = format_event(obj, label)
                        if output:
                            print(output)
                    except Exception:
                        pass
                    line = fh.readline()

            if not got_data:
                time.sleep(0.2)

            # Periodically scan for new subagent files (~5s interval)
            scan_counter += 1
            if not no_sub and scan_counter >= 25:
                scan_counter = 0
                for f in Path(sessions_dir).glob("*/*/*/*.jsonl"):
                    fp = str(f)
                    if fp in open_files:
                        continue
                    meta = get_session_meta(fp)
                    if meta and meta[1] in known_ids:
                        tid = meta[0]
                        print(f"\n{C_BOLD}{C_CYAN}>>> New subagent: {tid}{C_RESET}\n")
                        open_and_register(fp, f"sub:{short_id(tid)}", tid)
    except KeyboardInterrupt:
        pass
    finally:
        for fh, _ in open_files.values():
            fh.close()

# ── Entry point ─────────────────────────────────────────────────────
if __name__ == "__main__":
    main_file  = sys.argv[1]
    sessions_dir = sys.argv[2]
    follow     = sys.argv[3].lower() == "true"
    backfill   = int(sys.argv[4]) if sys.argv[4] else 0
    no_sub     = sys.argv[5].lower() == "true"

    sub_files = []
    if not no_sub:
        sub_files = discover_subagent_files(main_file, sessions_dir)
        if sub_files:
            print(f"{C_BOLD}Found {len(sub_files)} subagent(s):{C_RESET}")
            for fpath, tid in sub_files:
                print(f"  {C_DIM}{short_id(tid)} -> {Path(fpath).name}{C_RESET}")
            print()

    if follow:
        run_follow(main_file, sessions_dir, sub_files, backfill, no_sub)
    else:
        run_static(main_file, sub_files, backfill)
PYEOF

python3 "$PYFILE" "$LATEST" "$SESSIONS_DIR" "$FOLLOW" "$BACKFILL" "$NO_SUB"
