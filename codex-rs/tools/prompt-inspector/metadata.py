"""Git metadata and token counting utilities."""

from __future__ import annotations

import math
import subprocess
from concurrent.futures import ThreadPoolExecutor


def token_count(text: str) -> int:
    """Approximate token count: ceil(utf-8 byte length / 4)."""
    return math.ceil(len(text.encode("utf-8")) / 4)


def git_metadata(filepath: str, codex_root: str) -> dict:
    """Get git metadata for a single file.

    Runs `git log -1 --format="%ar|%an|%ai" -- {filepath}` and returns
    {"modified": "3 days ago", "author": "alice", "date": "2026-03-15T10:30:00+00:00"}.
    Returns empty dict on error.
    """
    try:
        result = subprocess.run(
            ["git", "log", "-1", "--format=%ar|%an|%ai", "--", filepath],
            capture_output=True,
            text=True,
            cwd=codex_root,
            timeout=10,
        )
        if result.returncode != 0 or not result.stdout.strip():
            return {}
        parts = result.stdout.strip().split("|", 2)
        if len(parts) != 3:
            return {}
        return {
            "modified": parts[0],
            "author": parts[1],
            "date": parts[2],
        }
    except (subprocess.SubprocessError, OSError):
        return {}


def git_metadata_batch(filepaths: list[str], codex_root: str) -> dict[str, dict]:
    """Get git metadata for multiple files in parallel.

    Uses a ThreadPoolExecutor with max_workers=16.
    Returns {filepath: metadata_dict} for each input filepath.
    """

    def _fetch(fp: str) -> tuple[str, dict]:
        return fp, git_metadata(fp, codex_root)

    results: dict[str, dict] = {}
    with ThreadPoolExecutor(max_workers=16) as pool:
        for fp, meta in pool.map(_fetch, filepaths):
            results[fp] = meta
    return results
