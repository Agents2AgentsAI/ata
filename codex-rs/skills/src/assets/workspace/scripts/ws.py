#!/usr/bin/env python3
"""
ws.py — Atomic workspace manifest helper for the $workspace skill.

Stdlib-only Python. Matches Rust WorkspaceService paths and flock(2) locking.

Subcommands:
  init <name>                              Create workspace, print ID
  list                                     List all workspaces as JSON array
  read [--workspace ID]                    Print manifest JSON
  mutate <jq_expr> [--workspace] [--expect-version N]
                                           Lock + jq mutate + version bump + atomic write
  select <id>                              Set active workspace selection (session-aware JSON)
  audit <json> [--workspace]               Append audit entry to audit.ndjson
  delete <id>                              Remove workspace directory tree
  resolve <@spec> [--workspace]            Resolve @-path alias to absolute path
  check-host <url> [--workspace]           Validate URL + check host allowlist
  run-locked --level <lvl> [--target-id]   Run command under fine-grained lock
  mirror-path <url>                        Print shared mirror cache path
  audit-query [--workspace] [--since] [--until] [--ops] [--limit]
                                           Query audit log as JSON array
  repo-clone <url> <alias> [--workspace] [--full]
                                           Validate, clone, register, audit (full repo_add flow)
  run-setup <name> --source-alias <alias> [--strategy STR] [--workspace]
                                           Create run dir, materialize code, register, audit
  recipe <operation>                       Print step-by-step recipe for an operation
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
    """Return the path to the active workspace selection file.

    Session-aware: if $CODEX_SESSION_ID is set, returns the session-scoped
    selection file. Otherwise returns the legacy global selection path.
    """
    sid = os.environ.get("CODEX_SESSION_ID", "").strip()
    if sid:
        return os.path.join(codex_home(), "sessions", sid, "workspace.json")
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


def _acquire_lock_on_path(lock_file: str):
    """Acquire exclusive flock on an arbitrary lock file. Returns fd."""
    os.makedirs(os.path.dirname(lock_file), exist_ok=True)
    fd = os.open(lock_file, os.O_RDWR | os.O_CREAT)
    deadline = time.monotonic() + LOCK_TIMEOUT_S
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fd
        except OSError:
            if time.monotonic() >= deadline:
                os.close(fd)
                _die(f"timeout acquiring lock after {LOCK_TIMEOUT_S}s: {lock_file}")
            time.sleep(LOCK_POLL_INTERVAL_S)


def _acquire_lock(workspace_id: str):
    """Acquire exclusive flock on workspace lock file. Returns fd."""
    return _acquire_lock_on_path(lock_path(workspace_id))


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
    expect_version = getattr(args, "expect_version", None)

    # Verify jq is available
    if not shutil.which("jq"):
        _die("jq is required but not found in PATH")

    fd = _acquire_lock(wid)
    try:
        manifest = _read_manifest(wid)

        # Version conflict check: exit 2 if manifest version != expected
        if expect_version is not None:
            actual = manifest.get("manifestVersion", 0)
            if actual != expect_version:
                print(
                    f"error: version conflict: expected {expect_version}, got {actual}",
                    file=sys.stderr,
                )
                sys.exit(2)

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
    selection = {
        "schemaVersion": 1,
        "activeWorkspaceId": wid,
        "updatedAt": _now(),
    }
    _atomic_write(sp, json.dumps(selection, indent=2).encode("utf-8"))
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
# Subcommand: check-host
# ---------------------------------------------------------------------------

# Patterns that indicate embedded credentials or tokens in URLs
_TOKEN_PATTERNS = re.compile(
    r"ghp_[A-Za-z0-9_]|gho_[A-Za-z0-9_]|github_pat_[A-Za-z0-9_]",
    re.IGNORECASE,
)
_QUERY_TOKEN_PARAMS = re.compile(
    r"[?&](token|access_token)=", re.IGNORECASE
)


def _validate_repo_url(url: str) -> None:
    """Reject non-HTTPS, embedded creds, and token patterns."""
    if not url.startswith("https://"):
        _die(f"only https:// URLs allowed, got: {url}")
    # Embedded credentials: user:pass@host
    if re.search(r"://[^/@]+:[^/@]+@", url):
        _die("embedded credentials not allowed in URL")
    # Token query params
    if _QUERY_TOKEN_PARAMS.search(url):
        _die("token query parameters not allowed in URL")
    # GitHub PAT patterns
    if _TOKEN_PATTERNS.search(url):
        _die("GitHub PAT/token patterns not allowed in URL")


def _extract_host(url: str) -> str:
    """Extract and normalize host from URL (lowercase, strip default port)."""
    # Strip scheme
    after_scheme = url.split("://", 1)[1] if "://" in url else url
    # Strip path/query
    host_part = after_scheme.split("/", 1)[0].split("?", 1)[0]
    # Strip userinfo (shouldn't be present after validation, but be safe)
    if "@" in host_part:
        host_part = host_part.rsplit("@", 1)[1]
    host_part = host_part.lower()
    # Strip default HTTPS port
    if host_part.endswith(":443"):
        host_part = host_part[:-4]
    return host_part


def cmd_check_host(args: argparse.Namespace) -> None:
    """Validate URL security and check against host allowlist."""
    url = args.url
    _validate_repo_url(url)

    wid = _resolve_workspace(args)
    manifest = _read_manifest(wid)
    policies = manifest.get("policies", {})
    allowlist = policies.get("repoHostsAllowlist")

    # None/absent allowlist = allow all hosts
    # Empty list [] = block all hosts
    if allowlist is not None:
        host = _extract_host(url)
        allowed_hosts = [h.lower() for h in allowlist]
        if host not in allowed_hosts:
            _die(f"host '{host}' not in allowlist: {allowed_hosts}")

    print(url)


# ---------------------------------------------------------------------------
# Subcommand: run-locked
# ---------------------------------------------------------------------------

_LOCK_LEVELS = {
    "workspace": lambda wid, _tid: os.path.join(
        workspace_root(wid), "locks", "workspace.lock"
    ),
    "kb": lambda wid, _tid: os.path.join(
        workspace_root(wid), "knowledge-base", "kb.lock"
    ),
    "run": lambda wid, tid: os.path.join(
        workspace_root(wid), "runs", tid, "run.lock"
    ),
    "index": lambda wid, tid: os.path.join(
        workspace_root(wid), "indexes", tid, "index.lock"
    ),
}


def cmd_run_locked(args: argparse.Namespace) -> None:
    """Acquire a fine-grained lock, run a command, release."""
    wid = _resolve_workspace(args)
    level = args.level
    target_id = getattr(args, "target_id", None)

    if level in ("run", "index") and not target_id:
        _die(f"--target-id is required for lock level '{level}'")

    # Strip leading '--' separator from REMAINDER
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        _die("no command specified for run-locked")

    lock_fn = _LOCK_LEVELS.get(level)
    if lock_fn is None:
        _die(f"unknown lock level: {level}")

    lf = lock_fn(wid, target_id)
    fd = _acquire_lock_on_path(lf)
    try:
        result = subprocess.run(command)
        sys.exit(result.returncode)
    finally:
        _release_lock(fd)


# ---------------------------------------------------------------------------
# Subcommand: mirror-path
# ---------------------------------------------------------------------------


def _normalize_repo_key(url: str) -> str:
    """Normalize a repo URL to a stable cache key.

    Strips scheme, trailing .git, lowercases, strips trailing slash.
    """
    # Strip scheme
    key = url.split("://", 1)[1] if "://" in url else url
    key = key.lower()
    # Strip trailing .git
    if key.endswith(".git"):
        key = key[:-4]
    # Strip trailing slash
    key = key.rstrip("/")
    return key


def cmd_mirror_path(args: argparse.Namespace) -> None:
    """Print the shared mirror cache path for a repo URL."""
    url = args.url
    key = _normalize_repo_key(url)
    digest = hashlib.sha256(key.encode("utf-8")).hexdigest()[:16]
    mirror_dir = os.path.join(codex_home(), "caches", "repo-mirrors", digest)
    print(mirror_dir)


# ---------------------------------------------------------------------------
# Subcommand: audit-query
# ---------------------------------------------------------------------------


def cmd_audit_query(args: argparse.Namespace) -> None:
    """Query audit log entries as JSON array. Pure Python NDJSON reader."""
    wid = _resolve_workspace(args)
    ap = audit_path(wid)

    since_ts = getattr(args, "since", None)
    until_ts = getattr(args, "until", None)
    ops_filter = getattr(args, "ops", None)
    limit = getattr(args, "limit", 200) or 200

    if ops_filter:
        ops_set = frozenset(o.strip() for o in ops_filter.split(",") if o.strip())
    else:
        ops_set = None

    results = []
    if not os.path.isfile(ap):
        print("[]")
        return

    with open(ap, "r") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue

            # Time filtering: parse ISO timestamp to unix
            ts_str = entry.get("ts", "")
            try:
                ts_unix = int(
                    time.mktime(time.strptime(ts_str, "%Y-%m-%dT%H:%M:%SZ"))
                    - time.timezone
                )
            except (ValueError, OverflowError):
                ts_unix = 0

            if since_ts is not None and ts_unix < since_ts:
                continue
            if until_ts is not None and ts_unix > until_ts:
                continue
            if ops_set is not None and entry.get("op") not in ops_set:
                continue

            results.append(entry)
            if len(results) >= limit:
                break

    print(json.dumps(results, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: repo-clone  (thick — full repo_add flow)
# ---------------------------------------------------------------------------


def _build_clone_args(policy: dict, full: bool) -> list:
    """Build git clone argument list from clone policy."""
    if full:
        return []
    args = []
    depth = policy.get("depth", 1)
    if isinstance(depth, int) and depth > 0:
        args += ["--depth", str(depth)]
    if policy.get("singleBranch", True):
        args.append("--single-branch")
    if policy.get("noTags", True):
        args.append("--no-tags")
    filt = policy.get("filter")
    if filt and filt != "null":
        args.append(f"--filter={filt}")
    if policy.get("submodules") == "recursive":
        args.append("--recurse-submodules")
    return args


def _read_git_state(repo_dir: str) -> dict:
    """Read HEAD sha, ref, and default branch from a cloned repo."""
    def _git(*cmd):
        r = subprocess.run(
            ["git", "-C", repo_dir] + list(cmd),
            capture_output=True, text=True,
        )
        return r.stdout.strip() if r.returncode == 0 else ""

    head_sha = _git("rev-parse", "HEAD")
    head_ref = _git("symbolic-ref", "--short", "HEAD")
    default_branch = _git("rev-parse", "--abbrev-ref", "origin/HEAD")
    if default_branch.startswith("origin/"):
        default_branch = default_branch[len("origin/"):]
    if not default_branch:
        default_branch = "main"
    return {
        "headSha": head_sha,
        "headRef": head_ref,
        "defaultBranch": default_branch,
    }


def _audit_entry(wid: str, op: str, targets: list, details: "dict | None" = None):
    """Build and write an audit entry."""
    entry = {"op": op, "targets": targets}
    if details:
        entry["details"] = details
    # Reuse the audit machinery
    actor = {"kind": "agent"}
    session_id = os.environ.get("CODEX_SESSION_ID")
    thread_id = os.environ.get("CODEX_THREAD_ID")
    if session_id:
        actor["sessionId"] = session_id
    if thread_id:
        actor["threadId"] = thread_id
    full_entry = {
        "schemaVersion": 1,
        "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "workspaceId": wid,
        "actor": actor,
        "op": op,
        "status": entry.get("status", "success"),
        "targets": targets,
    }
    if details:
        full_entry["details"] = details
    ap = audit_path(wid)
    os.makedirs(os.path.dirname(ap), exist_ok=True)
    line = json.dumps(full_entry, ensure_ascii=False) + "\n"
    with open(ap, "a") as f:
        f.write(line)
    return full_entry


def cmd_repo_clone(args: argparse.Namespace) -> None:
    """Full repo_add flow: validate URL → clone → register → audit."""
    wid = _resolve_workspace(args)
    url = args.url
    alias = args.alias
    full = getattr(args, "full", False)

    # Validate alias
    if alias in RESERVED_ALIASES:
        _die(f"'{alias}' is a reserved alias")
    if not re.fullmatch(r"[a-zA-Z0-9_-]+", alias):
        _die(f"invalid alias '{alias}': allowed chars [a-zA-Z0-9_-]")

    # 1. Validate URL (check-host logic)
    _validate_repo_url(url)
    manifest = _read_manifest(wid)
    policies = manifest.get("policies", {})
    allowlist = policies.get("repoHostsAllowlist")
    if allowlist is not None:
        host = _extract_host(url)
        allowed_hosts = [h.lower() for h in allowlist]
        if host not in allowed_hosts:
            _die(f"host '{host}' not in allowlist: {allowed_hosts}")

    # 2. Check alias not already used
    for repo in manifest.get("repos", []):
        if repo.get("alias") == alias:
            _die(f"alias '{alias}' already exists in workspace")

    # Capture version for optimistic concurrency
    version = manifest.get("manifestVersion", 0)

    # 3. Read clone policy and build args
    clone_policy = policies.get("defaultClone", {})
    clone_args = _build_clone_args(clone_policy, full)

    # 4. Clone
    root = workspace_root(wid)
    clone_dest = os.path.join(root, "repos", alias)
    if os.path.exists(clone_dest):
        _die(f"directory already exists: {clone_dest}")

    cmd = ["git", "clone"] + clone_args + [url, clone_dest]
    result = subprocess.run(cmd)
    if result.returncode != 0:
        _die(f"git clone failed (exit {result.returncode})")

    # 4b. LFS pull if policy says so
    lfs_policy = clone_policy.get("lfs", "auto")
    if lfs_policy in ("always", "auto"):
        subprocess.run(
            ["git", "-C", clone_dest, "lfs", "pull"],
            capture_output=True,
        )

    # 5. Read git state
    git_state = _read_git_state(clone_dest)
    depth = clone_policy.get("depth", 1) if not full else 0
    git_state["shallow"] = isinstance(depth, int) and depth > 0

    # 6. Build repo entry
    repo_id = _make_id("repo")

    # Derive repoKey from URL (org/repo)
    repo_key = url.rstrip("/")
    if repo_key.endswith(".git"):
        repo_key = repo_key[:-4]
    repo_key = repo_key.split("://", 1)[-1]  # strip scheme
    parts = repo_key.split("/")
    if len(parts) >= 2:
        repo_key = "/".join(parts[-2:])

    clone_record = {
        "depth": clone_policy.get("depth", 1) if not full else 0,
        "singleBranch": clone_policy.get("singleBranch", True) if not full else False,
        "noTags": clone_policy.get("noTags", True) if not full else False,
        "filter": clone_policy.get("filter", "blob:limit=1m") if not full else "",
        "submodules": clone_policy.get("submodules", "none"),
        "lfs": clone_policy.get("lfs", "auto"),
    }

    repo_entry = {
        "id": repo_id,
        "alias": alias,
        "repoKey": repo_key,
        "remoteUrl": url,
        "checkoutPath": f"repos/{alias}",
        "notesPath": f"notes/repos/{alias}",
        "clone": clone_record,
        "pin": {"mode": "tracking"},
        "state": git_state,
    }

    # 7. Mutate manifest (with optimistic concurrency)
    if not shutil.which("jq"):
        _die("jq is required but not found in PATH")

    fd = _acquire_lock(wid)
    try:
        manifest = _read_manifest(wid)
        actual = manifest.get("manifestVersion", 0)
        if actual != version:
            # Clean up the clone since we can't register it
            shutil.rmtree(clone_dest, ignore_errors=True)
            print(
                f"error: version conflict: expected {version}, got {actual}",
                file=sys.stderr,
            )
            sys.exit(2)
        manifest["repos"] = manifest.get("repos", []) + [repo_entry]
        manifest = _bump_version(manifest)
        _write_manifest(wid, manifest)
    finally:
        _release_lock(fd)

    # 8. Create notes dir
    notes_dir = os.path.join(root, "notes", "repos", alias)
    os.makedirs(notes_dir, exist_ok=True)

    # 9. Audit
    _audit_entry(wid, "repo_add", [
        {"type": "repo", "id": repo_id, "alias": alias},
    ])

    # 10. Output result
    result = {
        "repoId": repo_id,
        "alias": alias,
        "checkoutPath": f"repos/{alias}",
        "state": git_state,
    }
    print(json.dumps(result, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: run-setup  (thick — full run creation flow)
# ---------------------------------------------------------------------------


def cmd_run_setup(args: argparse.Namespace) -> None:
    """Full run creation: create dirs → materialize code → register → audit."""
    wid = _resolve_workspace(args)
    run_name = args.name
    source_alias = args.source_alias
    strategy = getattr(args, "strategy", "worktree") or "worktree"

    if strategy not in ("worktree", "copy", "clone"):
        _die(f"invalid strategy '{strategy}': must be worktree, copy, or clone")

    # 1. Verify source repo exists
    manifest = _read_manifest(wid)
    source_repo = None
    for repo in manifest.get("repos", []):
        if repo.get("alias") == source_alias:
            source_repo = repo
            break
    if source_repo is None:
        _die(f"source repo alias '{source_alias}' not found in workspace")

    version = manifest.get("manifestVersion", 0)
    root = workspace_root(wid)
    repo_path = os.path.join(root, "repos", source_alias)

    # 2. Create run structure
    run_id = _make_id("run")
    run_root = os.path.join(root, "runs", run_id)
    for sub in ["logs", "outputs", "tmp", "env"]:
        os.makedirs(os.path.join(run_root, sub), exist_ok=True)

    # 3. Materialize code
    code_root = os.path.join(run_root, "root")
    if strategy == "worktree":
        result = subprocess.run(
            ["git", "-C", repo_path, "worktree", "add", code_root, "HEAD"],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            # Fall back to copy if worktree fails
            shutil.copytree(repo_path, code_root)
            strategy = "copy"  # record actual strategy used
    elif strategy == "clone":
        result = subprocess.run(
            ["git", "clone", repo_path, code_root],
            capture_output=True, text=True,
        )
        if result.returncode != 0:
            shutil.rmtree(run_root, ignore_errors=True)
            _die(f"git clone failed: {result.stderr}")
    else:  # copy
        shutil.copytree(repo_path, code_root)

    # 4. Read HEAD SHA from materialized code
    head_sha = ""
    r = subprocess.run(
        ["git", "-C", code_root, "rev-parse", "HEAD"],
        capture_output=True, text=True,
    )
    if r.returncode == 0:
        head_sha = r.stdout.strip()

    # 5. Write run.json
    now = _now()
    run_manifest = {
        "schemaVersion": 2,
        "id": run_id,
        "name": run_name,
        "createdAt": now,
        "updatedAt": now,
        "status": "created",
        "source": {"repoAlias": source_alias, "sha": head_sha},
        "strategy": strategy,
        "commands": [],
        "env": {},
    }
    run_json_path = os.path.join(run_root, "run.json")
    _atomic_write(run_json_path, json.dumps(run_manifest, indent=2).encode("utf-8"))

    # 6. Register in workspace manifest
    if not shutil.which("jq"):
        _die("jq is required but not found in PATH")

    fd = _acquire_lock(wid)
    try:
        manifest = _read_manifest(wid)
        actual = manifest.get("manifestVersion", 0)
        if actual != version:
            shutil.rmtree(run_root, ignore_errors=True)
            print(
                f"error: version conflict: expected {version}, got {actual}",
                file=sys.stderr,
            )
            sys.exit(2)
        run_entry = {
            "id": run_id,
            "name": run_name,
            "createdAt": now,
            "updatedAt": now,
            "rootPath": f"runs/{run_id}",
            "status": "created",
            "source": {"repoAlias": source_alias},
        }
        manifest["runs"] = manifest.get("runs", []) + [run_entry]
        manifest = _bump_version(manifest)
        _write_manifest(wid, manifest)
    finally:
        _release_lock(fd)

    # 7. Audit
    _audit_entry(wid, "run_create", [
        {"type": "run", "id": run_id},
    ])

    # 8. Output result
    result = {
        "runId": run_id,
        "name": run_name,
        "rootPath": f"runs/{run_id}",
        "codePath": f"runs/{run_id}/root",
        "strategy": strategy,
        "source": {"repoAlias": source_alias, "sha": head_sha},
    }
    print(json.dumps(result, indent=2))


# ---------------------------------------------------------------------------
# Subcommand: recipe  (print step-by-step recipe for an operation)
# ---------------------------------------------------------------------------

_RECIPES = {
    "repo_update": """\
# Update repository to latest upstream
ALIAS="<repo_alias>"
REPO_PATH=$(python3 $WS resolve "@$ALIAS" --workspace "$WID")

git -C "$REPO_PATH" fetch --depth 1
DEFAULT=$(git -C "$REPO_PATH" rev-parse --abbrev-ref origin/HEAD | sed 's|origin/||')
git -C "$REPO_PATH" reset --hard "origin/$DEFAULT"

HEAD_SHA=$(git -C "$REPO_PATH" rev-parse HEAD)
NOW=$(date +%s)
python3 $WS mutate --workspace "$WID" \\
  ".repos = [.repos[] | if .alias == \\"$ALIAS\\" then .state.headSha = \\"$HEAD_SHA\\" | .state.lastUpdatedAt = $NOW else . end]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"repo_update","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'""",

    "repo_pin": """\
# Pin repository to a specific commit
ALIAS="<repo_alias>"
SHA="<commit_sha>"
python3 $WS mutate --workspace "$WID" \\
  ".repos = [.repos[] | if .alias == \\"$ALIAS\\" then .pin = {\\"mode\\":\\"pinned\\",\\"pinnedSha\\":\\"$SHA\\"} else . end]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"repo_pin","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'""",

    "repo_unpin": """\
# Unpin repository (switch back to tracking mode)
ALIAS="<repo_alias>"
python3 $WS mutate --workspace "$WID" \\
  ".repos = [.repos[] | if .alias == \\"$ALIAS\\" then .pin = {\\"mode\\":\\"tracking\\"} else . end]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"repo_track","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'""",

    "repo_remove": """\
# Remove repository from workspace
ALIAS="<repo_alias>"
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
rm -rf "$WS_ROOT/repos/$ALIAS"
python3 $WS mutate --workspace "$WID" \\
  ".repos = [.repos[] | select(.alias != \\"$ALIAS\\")]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"repo_remove","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'""",

    "run_exec": """\
# Execute a command in an existing run
RUN_ID="<run_id>"
RUN_ROOT=$(python3 $WS resolve "@run/$RUN_ID" --workspace "$WID")

# Mark as running
python3 $WS mutate --workspace "$WID" \\
  ".runs = [.runs[] | if .id == \\"$RUN_ID\\" then .status = \\"running\\" | .updatedAt = $(date +%s) else . end]"

# Execute with timeout and log capture
cd "$RUN_ROOT/root"
LOG_FILE="$RUN_ROOT/logs/$(date +%s).txt"
timeout 900 <command> 2>&1 | head -c 65536 | tee "$LOG_FILE"
EXIT_CODE=${PIPESTATUS[0]}

# Update status
STATUS=$( [ "$EXIT_CODE" -eq 0 ] && echo "completed" || echo "failed" )
python3 $WS mutate --workspace "$WID" \\
  '.runs = [.runs[] | if .id == "'"$RUN_ID"'" then .status = "'"$STATUS"'" | .updatedAt = '"$(date +%s)"' else . end]'""",

    "run_delete": """\
# Delete a run and clean up worktree
RUN_ID="<run_id>"
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')

# Clean up worktree if applicable
REPO_PATH=$(git -C "$WS_ROOT/runs/$RUN_ID/root" rev-parse --git-common-dir 2>/dev/null | xargs dirname 2>/dev/null || true)
if [ -n "$REPO_PATH" ]; then
  git -C "$REPO_PATH" worktree remove "$WS_ROOT/runs/$RUN_ID/root" --force 2>/dev/null || true
fi
rm -rf "$WS_ROOT/runs/$RUN_ID"
python3 $WS mutate --workspace "$WID" \\
  ".runs = [.runs[] | select(.id != \\"$RUN_ID\\")]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"run_delete","targets":[{"type":"run","id":"'"$RUN_ID"'"}]}'""",

    "run_gc": """\
# List stale runs (completed/failed, older than 7 days)
python3 $WS read --workspace "$WID" | jq -r \\
  '.runs[] | select(.status == "completed" or .status == "failed") | select(.updatedAt < (now - 604800)) | .id'
# Then delete each with the run_delete recipe""",

    "resource_add": """\
# Add a resource (paper, dataset, or artifact)
# Replace <type> with: papers, datasets, or artifacts
# Replace <prefix> with: paper, dataset, or artifact
TYPE="<type>"      # papers | datasets | artifacts
PREFIX="<prefix>"  # paper | dataset | artifact
RESOURCE_ID=$(python3 -c "import uuid,time; print(f'$PREFIX-{int(time.time())}-{uuid.uuid4().hex}')")
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')

python3 $WS mutate --workspace "$WID" \\
  ".$TYPE += [{\\"id\\":\\"$RESOURCE_ID\\",\\"title\\":\\"<title>\\",\\"notesPath\\":\\"notes/$TYPE/$RESOURCE_ID\\"}]"
mkdir -p "$WS_ROOT/notes/$TYPE/$RESOURCE_ID"
python3 $WS audit --workspace "$WID" \\
  '{"op":"'"${PREFIX}_add"'","targets":[{"type":"'"$PREFIX"'","id":"'"$RESOURCE_ID"'"}]}'

# Additional fields per type:
# Papers: "authors":[], "year":N, "doi":"", "arxiv":"", "url":"", "pdfArtifactId":""
# Datasets: "name":"", "url":"", "license":"", "artifactIds":[]
# Artifacts: "kind":"" (required: pdf|model|data), "sourceUrl":"", "sha256":"", "sizeBytes":N""",

    "link_add": """\
# Add a link between two resources
python3 $WS mutate --workspace "$WID" \\
  '.links += [{"from":{"type":"<from_type>","id":"<from_id>"},"to":{"type":"<to_type>","id":"<to_id>"},"kind":"<kind>"}]'
# Common kinds: implements, uses, produces, derived_from, related_to""",

    "snapshot_create": """\
# Create a manifest snapshot
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
SNAP_ID=$(python3 -c "import uuid,time; print(f'snap-{int(time.time())}-{uuid.uuid4().hex}')")
SNAP_PATH="notes/workspace/snapshots/$SNAP_ID.json"
cp "$WS_ROOT/workspace.json" "$WS_ROOT/$SNAP_PATH"
python3 $WS mutate --workspace "$WID" \\
  ".snapshots += [{\\"id\\":\\"$SNAP_ID\\",\\"path\\":\\"$SNAP_PATH\\"}]"
python3 $WS audit --workspace "$WID" \\
  '{"op":"snapshot_create","targets":[{"type":"snapshot","id":"'"$SNAP_ID"'"}]}'""",

    "snapshot_restore": """\
# Restore repo pins from a snapshot (additive — never deletes repos)
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
SNAP_PATH="<path_to_snapshot>"  # e.g. notes/workspace/snapshots/snap-XXX.json

# For each repo in snapshot:
jq -c '.repos[]' "$WS_ROOT/$SNAP_PATH" | while read -r REPO; do
  ALIAS=$(echo "$REPO" | jq -r '.alias')
  PIN_SHA=$(echo "$REPO" | jq -r '.pin.pinnedSha // empty')
  # Skip if alias already exists (or handle per policy)
  # Checkout pinned SHA if present:
  if [ -n "$PIN_SHA" ]; then
    git -C "$WS_ROOT/repos/$ALIAS" fetch origin "$PIN_SHA" --depth 1 2>/dev/null || true
    git -C "$WS_ROOT/repos/$ALIAS" checkout "$PIN_SHA" 2>/dev/null || true
  fi
  python3 $WS mutate --workspace "$WID" \\
    ".repos = [.repos[] | if .alias == \\"$ALIAS\\" then .pin = $(echo "$REPO" | jq '.pin') else . end]"
done""",

    "export": """\
# Export workspace bundle
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
BUNDLE_PATH="/tmp/$WID-export.tar.gz"

# Sanitize URLs before export
python3 $WS read --workspace "$WID" | \\
  jq '.repos = [.repos[] | .remoteUrl = (.remoteUrl | split("?")[0])]' \\
  > "/tmp/$WID-export-manifest.json"

# Build file list (customize modes: notes, runs, artifacts, repos)
tar -czf "$BUNDLE_PATH" -C "$WS_ROOT" \\
  workspace.json notes/workspace/ knowledge-base/
echo "Exported to: $BUNDLE_PATH" """,

    "import": """\
# Import workspace bundle
NEW_WID=$(python3 $WS init "Imported Project")
NEW_ROOT=$(python3 $WS resolve '@ws' --workspace "$NEW_WID" | sed 's|/$||')
tar -xzf "$BUNDLE_PATH" -C "$NEW_ROOT"
python3 $WS mutate --workspace "$NEW_WID" ".id = \\"$NEW_WID\\""
echo "Imported as: $NEW_WID" """,

    "index_build": """\
# Build a search index (scaffold — uses external indexing tool)
INDEX_ID=$(python3 -c "import uuid,time; print(f'idx-{int(time.time())}-{uuid.uuid4().hex}')")
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
INDEX_PATH="indexes/$INDEX_ID"
mkdir -p "$WS_ROOT/$INDEX_PATH"
NOW=$(date +%s)
python3 $WS mutate --workspace "$WID" \\
  ".indexes += [{\\"id\\":\\"$INDEX_ID\\",\\"kind\\":\\"keyword\\",\\"targetType\\":\\"repo\\",\\"targetId\\":\\"$REPO_ID\\",\\"createdAt\\":$NOW,\\"status\\":\\"building\\",\\"path\\":\\"$INDEX_PATH\\"}]"
# ... run external indexing tool, then mark ready:
python3 $WS mutate --workspace "$WID" \\
  ".indexes = [.indexes[] | if .id == \\"$INDEX_ID\\" then .status = \\"ready\\" | .updatedAt = $(date +%s) else . end]" """,
}


def cmd_recipe(args: argparse.Namespace) -> None:
    """Print step-by-step recipe for an operation."""
    op = args.operation
    if op == "list":
        print("Available recipes:")
        for name in sorted(_RECIPES):
            print(f"  {name}")
        return

    recipe = _RECIPES.get(op)
    if recipe is None:
        print(f"error: unknown recipe '{op}'", file=sys.stderr)
        print(f"Available: {', '.join(sorted(_RECIPES))}", file=sys.stderr)
        sys.exit(1)

    print(recipe)


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


def _read_workspace_selection_file(path: str) -> "str | None":
    """Read a workspace ID from a selection file.

    Handles two formats:
    - Structured JSON: {"schemaVersion":1,"activeWorkspaceId":"..."}
    - Legacy bare ID string (backward compat)

    Returns None if file missing, malformed, or referenced workspace doesn't exist.
    """
    if not os.path.isfile(path):
        return None
    try:
        raw = open(path, "r").read().strip()
        if not raw:
            return None
        # Try structured JSON first
        try:
            data = json.loads(raw)
            if isinstance(data, dict):
                wid = data.get("activeWorkspaceId", "")
            else:
                wid = ""
        except json.JSONDecodeError:
            # Legacy: bare workspace ID string
            wid = raw
        if wid and os.path.isfile(manifest_path(wid)):
            return wid
    except OSError:
        pass
    return None


def _discover_project_pin(cwd: str) -> "str | None":
    """Walk ancestors from cwd looking for .codex/workspace.json.

    Stops at .git boundary or filesystem root.
    """
    current = os.path.realpath(cwd)
    while True:
        candidate = os.path.join(current, ".codex", "workspace.json")
        wid = _read_workspace_selection_file(candidate)
        if wid is not None:
            return wid
        # Stop at .git boundary
        if os.path.isdir(os.path.join(current, ".git")):
            break
        parent = os.path.dirname(current)
        if parent == current:
            break
        current = parent
    return None


def _read_session_workspace() -> "str | None":
    """Read workspace from session selection file.

    Looks at $CODEX_HOME/sessions/$CODEX_SESSION_ID/workspace.json.
    Returns None if CODEX_SESSION_ID unset or file missing/invalid.
    """
    sid = os.environ.get("CODEX_SESSION_ID", "").strip()
    if not sid:
        return None
    path = os.path.join(codex_home(), "sessions", sid, "workspace.json")
    return _read_workspace_selection_file(path)


def _resolve_workspace(args: argparse.Namespace) -> str:
    """4-tier workspace resolution: explicit > project pin > session > global."""
    # 1. Explicit --workspace flag
    wid = getattr(args, "workspace", None)
    if wid:
        return wid

    # 2. Project pin (.codex/workspace.json from cwd upward)
    wid = _discover_project_pin(os.getcwd())
    if wid:
        return wid

    # 3. Session selection
    wid = _read_session_workspace()
    if wid:
        return wid

    # 4. Global fallback
    _ensure_global_workspace()
    return "global"


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
    p_mutate.add_argument(
        "--expect-version",
        type=int,
        default=None,
        help="Fail with exit code 2 if manifest version != this value",
    )

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

    # check-host
    p_check_host = sub.add_parser(
        "check-host", help="Validate repo URL and check host allowlist"
    )
    p_check_host.add_argument("url", help="Repository URL to validate")
    p_check_host.add_argument("--workspace", help="Workspace ID (default: global)")

    # run-locked
    p_run_locked = sub.add_parser(
        "run-locked", help="Run command under fine-grained lock"
    )
    p_run_locked.add_argument(
        "--level",
        required=True,
        choices=["workspace", "kb", "run", "index"],
        help="Lock level",
    )
    p_run_locked.add_argument("--workspace", help="Workspace ID (default: global)")
    p_run_locked.add_argument(
        "--target-id", help="Target ID (required for run/index levels)"
    )
    p_run_locked.add_argument(
        "command", nargs=argparse.REMAINDER, help="Command to run under lock"
    )

    # mirror-path
    p_mirror_path = sub.add_parser(
        "mirror-path", help="Print shared mirror cache path for a repo URL"
    )
    p_mirror_path.add_argument("url", help="Repository URL")

    # audit-query
    p_audit_query = sub.add_parser(
        "audit-query", help="Query audit log entries as JSON"
    )
    p_audit_query.add_argument("--workspace", help="Workspace ID (default: global)")
    p_audit_query.add_argument(
        "--since", type=int, default=None, help="Filter: entries after unix timestamp"
    )
    p_audit_query.add_argument(
        "--until", type=int, default=None, help="Filter: entries before unix timestamp"
    )
    p_audit_query.add_argument(
        "--ops", default=None, help="Filter: comma-separated operation names"
    )
    p_audit_query.add_argument(
        "--limit", type=int, default=200, help="Max entries to return (default: 200)"
    )

    # repo-clone (thick command)
    p_repo_clone = sub.add_parser(
        "repo-clone",
        help="Validate, clone, register, and audit a repo (full repo_add flow)",
    )
    p_repo_clone.add_argument("url", help="Repository URL (https:// only)")
    p_repo_clone.add_argument("alias", help="Local alias for the repo")
    p_repo_clone.add_argument("--workspace", help="Workspace ID (default: global)")
    p_repo_clone.add_argument(
        "--full",
        action="store_true",
        help="Full clone (ignore depth/filter policy)",
    )

    # run-setup (thick command)
    p_run_setup = sub.add_parser(
        "run-setup",
        help="Create run directory, materialize code, register, and audit",
    )
    p_run_setup.add_argument("name", help="Human-readable run name")
    p_run_setup.add_argument(
        "--source-alias", required=True, help="Source repo alias"
    )
    p_run_setup.add_argument(
        "--strategy",
        choices=["worktree", "copy", "clone"],
        default="worktree",
        help="Code materialization strategy (default: worktree)",
    )
    p_run_setup.add_argument("--workspace", help="Workspace ID (default: global)")

    # recipe
    p_recipe = sub.add_parser(
        "recipe",
        help="Print step-by-step recipe for an operation (use 'list' to see all)",
    )
    p_recipe.add_argument(
        "operation",
        help="Operation name (e.g., repo_update, run_exec) or 'list'",
    )

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
        "check-host": cmd_check_host,
        "run-locked": cmd_run_locked,
        "mirror-path": cmd_mirror_path,
        "audit-query": cmd_audit_query,
        "repo-clone": cmd_repo_clone,
        "run-setup": cmd_run_setup,
        "recipe": cmd_recipe,
    }

    handler = dispatch.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()
        sys.exit(1)


if __name__ == "__main__":
    main()
