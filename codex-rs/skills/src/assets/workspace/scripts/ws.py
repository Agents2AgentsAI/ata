#!/usr/bin/env python3
"""
ws.py — Atomic workspace manifest helper for the $workspace skill.

Stdlib-only Python. Matches Rust WorkspaceService paths and flock(2) locking.

Subcommands:
  init <name>                    Create workspace, print ID
  list                           List all workspaces as JSON array
  read [--workspace ID]          Print manifest JSON
  mutate <jq_expr> [--workspace] Lock + jq mutate + version bump + atomic write
  select <id>                    Set active workspace selection
  audit <json> [--workspace]     Append audit entry to audit.ndjson
  delete <id>                    Remove workspace directory tree
  resolve <@spec> [--workspace]  Resolve @-path alias to absolute path
"""

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

MAX_WORKSPACE_ID_LEN = 96
MANIFEST_FILENAME = "workspace.json"
LOCK_TIMEOUT_S = 30
LOCK_POLL_INTERVAL_S = 0.05
SCHEMA_VERSION = 2

RESERVED_ALIASES = frozenset(
    ["run", "notes", "ws", "artifacts", "kb", "index", "cache"]
)

NOTES_CATEGORIES = frozenset(
    ["workspace", "repos", "papers", "datasets", "runs", "artifacts", "indexes"]
)

# ---------------------------------------------------------------------------
# Helpers — paths
# ---------------------------------------------------------------------------


def codex_home() -> str:
    return os.environ.get("CODEX_HOME") or os.path.expanduser("~/.ata")


def workspaces_root() -> str:
    return os.path.join(codex_home(), "workspaces")


def workspace_root(workspace_id: str) -> str:
    return os.path.join(workspaces_root(), workspace_id)


def manifest_path(workspace_id: str) -> str:
    return os.path.join(workspace_root(workspace_id), MANIFEST_FILENAME)


def lock_path(workspace_id: str) -> str:
    return os.path.join(workspace_root(workspace_id), "locks", "workspace.lock")


def audit_path(workspace_id: str) -> str:
    return os.path.join(
        workspace_root(workspace_id), "notes", "workspace", "audit.ndjson"
    )


def selection_path() -> str:
    return os.path.join(codex_home(), ".workspace_selected")


# ---------------------------------------------------------------------------
# Helpers — workspace ID generation (matches Rust WorkspaceService)
# ---------------------------------------------------------------------------


def _slugify_name(name: str) -> str:
    chars = []
    for ch in name:
        if ch.isascii() and ch.isalnum():
            chars.append(ch.lower())
        else:
            chars.append("-")
    raw = "".join(chars)
    segments = [s for s in raw.split("-") if s]
    return "-".join(segments) if segments else "workspace"


def _short_hash(name: str) -> str:
    h = hashlib.sha256()
    h.update(name.encode("utf-8"))
    h.update(str(int(time.time())).encode("utf-8"))
    return h.hexdigest()[:8]


def _workspace_id_from_name(name: str, attempt: int = 0) -> str:
    hash_val = _short_hash(name)
    suffix = f"-{hash_val}" if attempt == 0 else f"-{hash_val}-{attempt}"
    max_slug = MAX_WORKSPACE_ID_LEN - len(suffix)
    slug = _slugify_name(name)
    if len(slug) > max_slug:
        slug = slug[:max_slug].strip("-")
    if not slug:
        slug = "workspace"
    return f"{slug}{suffix}"


def _validate_workspace_id(workspace_id: str) -> None:
    if not workspace_id:
        _die("workspace id must not be empty")
    if workspace_id in (".", ".."):
        _die("workspace id must not be '.' or '..'")
    if len(workspace_id) > MAX_WORKSPACE_ID_LEN:
        _die(f"workspace id exceeds {MAX_WORKSPACE_ID_LEN} characters")
    if workspace_id[0] in "-_" or workspace_id[-1] in "-_":
        _die(f"workspace id '{workspace_id}' must not start or end with '-' or '_'")
    if not re.fullmatch(r"[a-z0-9_-]+", workspace_id):
        _die(
            f"workspace id '{workspace_id}' contains invalid characters; allowed: [a-z0-9_-]"
        )


# ---------------------------------------------------------------------------
# Helpers — file locking (flock, matches Rust FileLock)
# ---------------------------------------------------------------------------


def _acquire_lock(workspace_id: str):
    """Acquire exclusive flock on workspace lock file. Returns (fd, path)."""
    lp = lock_path(workspace_id)
    os.makedirs(os.path.dirname(lp), exist_ok=True)
    fd = os.open(lp, os.O_RDWR | os.O_CREAT)
    deadline = time.monotonic() + LOCK_TIMEOUT_S
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fd
        except OSError:
            if time.monotonic() >= deadline:
                os.close(fd)
                _die(f"timeout acquiring workspace lock after {LOCK_TIMEOUT_S}s")
            time.sleep(LOCK_POLL_INTERVAL_S)


def _release_lock(fd: int) -> None:
    try:
        fcntl.flock(fd, fcntl.LOCK_UN)
    finally:
        os.close(fd)


# ---------------------------------------------------------------------------
# Helpers — atomic file write (matches Rust io_utils::atomic_write)
# ---------------------------------------------------------------------------


def _atomic_write(path: str, data: bytes) -> None:
    """Write data atomically: write to temp file, fsync, rename, fsync dir."""
    parent = os.path.dirname(path)
    os.makedirs(parent, exist_ok=True)
    tmp_name = os.path.join(parent, f".tmp-{uuid.uuid4()}")
    fd = os.open(tmp_name, os.O_WRONLY | os.O_CREAT | os.O_TRUNC)
    try:
        os.write(fd, data)
        os.fsync(fd)
    finally:
        os.close(fd)
    os.rename(tmp_name, path)
    # fsync parent directory
    dir_fd = os.open(parent, os.O_RDONLY)
    try:
        os.fsync(dir_fd)
    finally:
        os.close(dir_fd)


# ---------------------------------------------------------------------------
# Helpers — manifest I/O
# ---------------------------------------------------------------------------


def _read_manifest(workspace_id: str) -> dict:
    mp = manifest_path(workspace_id)
    if not os.path.isfile(mp):
        _die(f"workspace manifest not found: {mp}")
    with open(mp, "r") as f:
        return json.load(f)


def _write_manifest(workspace_id: str, manifest: dict) -> None:
    mp = manifest_path(workspace_id)
    data = json.dumps(manifest, indent=2, ensure_ascii=False).encode("utf-8")
    _atomic_write(mp, data)


def _bump_version(manifest: dict) -> dict:
    manifest["manifestVersion"] = manifest.get("manifestVersion", 0) + 1
    manifest["updatedAt"] = int(time.time())
    return manifest


# ---------------------------------------------------------------------------
# Helpers — path safety (matches Rust path_spec.rs:safe_join)
# ---------------------------------------------------------------------------


def _validate_path_suffix(suffix: str) -> None:
    """Reject path suffixes that attempt traversal."""
    if not suffix:
        return
    if os.path.isabs(suffix):
        _die(f"path suffix must be relative, got: {suffix}")
    for component in suffix.replace("\\", "/").split("/"):
        if component == "..":
            _die(f"path suffix must not contain '..': {suffix}")


def _safe_join(root: str, suffix: str) -> str:
    """Join root + suffix and verify result stays under root."""
    _validate_path_suffix(suffix)
    if not suffix:
        return root
    joined = os.path.join(root, suffix)
    real_root = os.path.realpath(root)
    real_joined = os.path.realpath(joined)
    if not real_joined.startswith(real_root + os.sep) and real_joined != real_root:
        _die(f"resolved path escapes workspace root: {suffix}")
    return joined


# ---------------------------------------------------------------------------
# Helpers — misc
# ---------------------------------------------------------------------------


def _die(msg: str, code: int = 1) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(code)


def _now() -> int:
    return int(time.time())


def _make_id(prefix: str) -> str:
    return f"{prefix}-{_now()}-{uuid.uuid4().hex}"


def _new_manifest(workspace_id: str, name: str) -> dict:
    """Build a fresh workspace manifest."""
    now = _now()
    return {
        "schemaVersion": SCHEMA_VERSION,
        "id": workspace_id,
        "name": name,
        "createdAt": now,
        "updatedAt": now,
        "manifestVersion": 1,
        "repos": [],
        "runs": [],
        "papers": [],
        "datasets": [],
        "artifacts": [],
        "links": [],
        "snapshots": [],
        "indexes": [],
        "policies": {
            "defaultClone": {
                "depth": 1,
                "singleBranch": True,
                "noTags": True,
                "filter": "blob:limit=1m",
                "submodules": "none",
                "lfs": "auto",
            }
        },
        "knowledgeBase": {"path": "knowledge-base"},
        "labels": {},
    }


# ---------------------------------------------------------------------------
# Subcommand: init
# ---------------------------------------------------------------------------

INIT_DIRS = [
    "",
    "repos",
    "runs",
    "artifacts",
    "indexes",
    "cache",
    "locks",
    os.path.join("notes", "workspace"),
    os.path.join("notes", "workspace", "snapshots"),
    os.path.join("notes", "repos"),
    os.path.join("notes", "papers"),
    os.path.join("notes", "datasets"),
    os.path.join("notes", "artifacts"),
    os.path.join("notes", "runs"),
    os.path.join("notes", "indexes"),
    "knowledge-base",
    os.path.join("knowledge-base", "cards"),
    os.path.join("knowledge-base", "topics"),
    os.path.join("knowledge-base", "briefings"),
    os.path.join("knowledge-base", "explanations"),
    os.path.join("knowledge-base", "assets"),
    os.path.join("knowledge-base", "staging"),
]


def cmd_init(args: argparse.Namespace) -> None:
    name = args.name
    attempt = 0
    while True:
        workspace_id = _workspace_id_from_name(name, attempt)
        _validate_workspace_id(workspace_id)
        root = workspace_root(workspace_id)
        if not os.path.exists(root):
            break
        attempt += 1

    for sub in INIT_DIRS:
        os.makedirs(os.path.join(root, sub), exist_ok=True)

    _write_manifest(workspace_id, _new_manifest(workspace_id, name))
    print(workspace_id)


# ---------------------------------------------------------------------------
# Subcommand: list
# ---------------------------------------------------------------------------


def cmd_list(_args: argparse.Namespace) -> None:
    root = workspaces_root()
    if not os.path.isdir(root):
        print("[]")
        return

    results = []
    for entry in sorted(os.listdir(root)):
        mp = os.path.join(root, entry, MANIFEST_FILENAME)
        if not os.path.isfile(mp):
            continue
        try:
            with open(mp, "r") as f:
                m = json.load(f)
            results.append(
                {
                    "id": m.get("id", entry),
                    "name": m.get("name", ""),
                    "updatedAt": m.get("updatedAt", 0),
                    "repoCount": len(m.get("repos", [])),
                }
            )
        except (json.JSONDecodeError, OSError):
            continue

    print(json.dumps(results, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: read
# ---------------------------------------------------------------------------


def cmd_read(args: argparse.Namespace) -> None:
    wid = _resolve_workspace(args)
    manifest = _read_manifest(wid)
    print(json.dumps(manifest, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: mutate
# ---------------------------------------------------------------------------


def cmd_mutate(args: argparse.Namespace) -> None:
    wid = _resolve_workspace(args)
    jq_expr = args.jq_expr

    # Verify jq is available
    if not shutil.which("jq"):
        _die("jq is required but not found in PATH")

    fd = _acquire_lock(wid)
    try:
        manifest = _read_manifest(wid)
        input_json = json.dumps(manifest, ensure_ascii=False)
        result = subprocess.run(
            ["jq", jq_expr],
            input=input_json,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            _die(f"jq failed: {result.stderr.strip()}")

        try:
            mutated = json.loads(result.stdout)
        except json.JSONDecodeError:
            _die(f"jq output is not valid JSON: {result.stdout[:200]}")

        mutated = _bump_version(mutated)
        _write_manifest(wid, mutated)
        print(json.dumps(mutated, indent=2))
    finally:
        _release_lock(fd)


# ---------------------------------------------------------------------------
# Subcommand: select
# ---------------------------------------------------------------------------


def cmd_select(args: argparse.Namespace) -> None:
    wid = args.id
    # Verify workspace exists
    if not os.path.isfile(manifest_path(wid)):
        _die(f"workspace '{wid}' not found")
    sp = selection_path()
    os.makedirs(os.path.dirname(sp), exist_ok=True)
    _atomic_write(sp, wid.encode("utf-8"))
    print(f"selected: {wid}")


# ---------------------------------------------------------------------------
# Subcommand: audit
# ---------------------------------------------------------------------------


def cmd_audit(args: argparse.Namespace) -> None:
    wid = _resolve_workspace(args)
    try:
        entry = json.loads(args.json_str)
    except json.JSONDecodeError as exc:
        _die(f"invalid JSON: {exc}")

    # Build audit actor with optional session/thread context
    actor = {"kind": "agent"}
    session_id = os.environ.get("CODEX_SESSION_ID")
    thread_id = os.environ.get("CODEX_THREAD_ID")
    if session_id:
        actor["sessionId"] = session_id
    if thread_id:
        actor["threadId"] = thread_id

    # Build full audit entry with envelope fields
    full_entry = {
        "schemaVersion": 1,
        "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "workspaceId": wid,
        "actor": actor,
        "op": entry.get("op", "unknown"),
        "status": entry.get("status", "success"),
        "targets": entry.get("targets", []),
    }
    if "details" in entry:
        full_entry["details"] = entry["details"]

    ap = audit_path(wid)
    os.makedirs(os.path.dirname(ap), exist_ok=True)
    line = json.dumps(full_entry, ensure_ascii=False) + "\n"
    with open(ap, "a") as f:
        f.write(line)
    print(json.dumps(full_entry, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: delete
# ---------------------------------------------------------------------------


def cmd_delete(args: argparse.Namespace) -> None:
    wid = args.id
    if wid == "global":
        _die("cannot delete the global workspace")
    root = workspace_root(wid)
    if not os.path.isdir(root):
        _die(f"workspace '{wid}' not found")
    shutil.rmtree(root)
    print(f"deleted: {wid}")


# ---------------------------------------------------------------------------
# Subcommand: resolve
# ---------------------------------------------------------------------------


def cmd_resolve(args: argparse.Namespace) -> None:
    wid = _resolve_workspace(args)
    spec = args.spec
    if not spec.startswith("@"):
        _die("path spec must start with '@'")

    root = workspace_root(wid)
    rest = spec[1:]  # strip leading @

    # @ws/<other_workspace_id>/path...
    if rest.startswith("ws/"):
        parts = rest.split("/", 2)
        if len(parts) < 2:
            _die("@ws requires a workspace id: @ws/<id>[/path]")
        target_wid = parts[1]
        suffix = parts[2] if len(parts) > 2 else ""
        print(_safe_join(workspace_root(target_wid), suffix))
        return

    # @run/<run_id>/path...
    if rest.startswith("run/"):
        parts = rest.split("/", 2)
        if len(parts) < 2:
            _die("@run requires a run id: @run/<id>[/path]")
        run_id = parts[1]
        suffix = parts[2] if len(parts) > 2 else ""
        run_root = os.path.join(root, "runs", run_id)
        print(_safe_join(run_root, suffix))
        return

    # @notes/path...
    if rest.startswith("notes"):
        suffix = rest[len("notes") :].lstrip("/")
        if not suffix:
            print(os.path.join(root, "notes", "workspace"))
        else:
            first = suffix.split("/")[0]
            if first in NOTES_CATEGORIES:
                print(_safe_join(os.path.join(root, "notes"), suffix))
            else:
                print(_safe_join(os.path.join(root, "notes", "workspace"), suffix))
        return

    # @kb/path...
    if rest.startswith("kb"):
        suffix = rest[len("kb") :].lstrip("/")
        print(_safe_join(os.path.join(root, "knowledge-base"), suffix))
        return

    # @cache/path...
    if rest.startswith("cache"):
        suffix = rest[len("cache") :].lstrip("/")
        print(_safe_join(os.path.join(root, "cache"), suffix))
        return

    # @artifacts/<id>/path...
    if rest.startswith("artifacts/"):
        suffix = rest[len("artifacts/") :]
        print(_safe_join(os.path.join(root, "artifacts"), suffix))
        return

    # @index/<id>/path...
    if rest.startswith("index/"):
        suffix = rest[len("index/") :]
        print(_safe_join(os.path.join(root, "indexes"), suffix))
        return

    # @<alias>/path... — repo alias
    parts = rest.split("/", 1)
    alias = parts[0]
    if alias in RESERVED_ALIASES:
        _die(f"'{alias}' is a reserved alias")
    suffix = parts[1] if len(parts) > 1 else ""
    print(_safe_join(os.path.join(root, "repos", alias), suffix))


# ---------------------------------------------------------------------------
# Workspace resolution
# ---------------------------------------------------------------------------


def _ensure_global_workspace() -> None:
    """Create the global workspace if it doesn't exist yet."""
    wid = "global"
    if os.path.isfile(manifest_path(wid)):
        return
    root = workspace_root(wid)
    for sub in INIT_DIRS:
        os.makedirs(os.path.join(root, sub), exist_ok=True)
    _write_manifest(wid, _new_manifest(wid, "global"))


def _resolve_workspace(args: argparse.Namespace) -> str:
    """Resolve workspace ID: explicit --workspace > 'global' fallback."""
    wid = getattr(args, "workspace", None)
    if wid:
        return wid
    wid = "global"
    _ensure_global_workspace()
    return wid


# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="ws.py",
        description="Atomic workspace manifest helper",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    # init
    p_init = sub.add_parser("init", help="Create a new workspace")
    p_init.add_argument("name", help="Workspace name")

    # list
    sub.add_parser("list", help="List all workspaces")

    # read
    p_read = sub.add_parser("read", help="Read workspace manifest")
    p_read.add_argument("--workspace", help="Workspace ID (default: global)")

    # mutate
    p_mutate = sub.add_parser("mutate", help="Atomically mutate manifest via jq")
    p_mutate.add_argument("jq_expr", help="jq expression to apply")
    p_mutate.add_argument("--workspace", help="Workspace ID (default: global)")

    # select
    p_select = sub.add_parser("select", help="Set active workspace")
    p_select.add_argument("id", help="Workspace ID to select")

    # audit
    p_audit = sub.add_parser("audit", help="Append audit entry")
    p_audit.add_argument("json_str", help="JSON audit entry")
    p_audit.add_argument("--workspace", help="Workspace ID (default: global)")

    # delete
    p_delete = sub.add_parser("delete", help="Delete workspace")
    p_delete.add_argument("id", help="Workspace ID to delete")

    # resolve
    p_resolve = sub.add_parser("resolve", help="Resolve @-path alias")
    p_resolve.add_argument("spec", help="Path spec (e.g., @repo/file.txt)")
    p_resolve.add_argument("--workspace", help="Workspace ID (default: global)")

    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    dispatch = {
        "init": cmd_init,
        "list": cmd_list,
        "read": cmd_read,
        "mutate": cmd_mutate,
        "select": cmd_select,
        "audit": cmd_audit,
        "delete": cmd_delete,
        "resolve": cmd_resolve,
    }

    handler = dispatch.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
