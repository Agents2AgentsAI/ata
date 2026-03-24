"""Extracts string content from different source types in a Rust codebase."""

from __future__ import annotations

import re


def extract_file_content(filepath: str) -> str:
    """Read the entire content of a .md, .txt, .xml, or .toml file."""
    with open(filepath, encoding="utf-8") as f:
        return f.read()


def _extract_regular_string(text: str, start: int) -> str | None:
    """Extract a regular double-quoted Rust string starting at position start (the opening quote)."""
    i = start + 1
    result: list[str] = []
    while i < len(text):
        ch = text[i]
        if ch == "\\":
            if i + 1 < len(text):
                next_ch = text[i + 1]
                if next_ch == "n":
                    result.append("\n")
                elif next_ch == "t":
                    result.append("\t")
                elif next_ch == "\\":
                    result.append("\\")
                elif next_ch == '"':
                    result.append('"')
                elif next_ch == "r":
                    result.append("\r")
                elif next_ch == "0":
                    result.append("\0")
                else:
                    result.append(next_ch)
                i += 2
                continue
            else:
                return None
        elif ch == '"':
            return "".join(result)
        else:
            result.append(ch)
            i += 1
    return None


def _extract_raw_string(text: str, start: int) -> str | None:
    """Extract a raw Rust string r#"..."# starting at the 'r' character."""
    i = start + 1  # skip 'r'
    hashes = 0
    while i < len(text) and text[i] == "#":
        hashes += 1
        i += 1
    if i >= len(text) or text[i] != '"':
        return None
    i += 1  # skip opening quote
    closing = '"' + "#" * hashes
    end = text.find(closing, i)
    if end == -1:
        return None
    return text[i:end]


def _find_string_at(text: str, pos: int) -> str | None:
    """Try to extract a Rust string literal starting at pos. Handles both regular and raw strings."""
    if pos >= len(text):
        return None
    if text[pos] == '"':
        return _extract_regular_string(text, pos)
    if text[pos] == "r" and pos + 1 < len(text) and text[pos + 1] in ('#', '"'):
        return _extract_raw_string(text, pos)
    return None


def _concat_adjacent_strings(text: str, pos: int) -> str | None:
    """Extract a string and any adjacent strings joined with whitespace/newlines (Rust string concat in macros)."""
    result = _find_string_at(text, pos)
    if result is None:
        return None
    return result


def extract_rust_const(filepath: str, pattern: str) -> tuple[str | None, int]:
    """Find a const/static matching `pattern` in a .rs file and extract its string value.

    For constants like `const FOO: &str = "..."`, extracts the string.
    For `LazyLock<ToolSpec>` or `ToolSpec { ... }` structs, locates the
    `description:` field and extracts its string value.
    Handles `"..."`, `r#"..."#`, and multi-line strings.

    Returns (content, line_number) or (None, 0) on failure.
    """
    try:
        with open(filepath, encoding="utf-8") as f:
            source = f.read()
    except OSError:
        return None, 0

    lines = source.split("\n")

    # Strategy 1: simple const/static with &str = "..." or = r#"..."#
    const_pat = re.compile(
        rf"""(?:const|static|pub\s+const|pub\s+static|pub\((?:crate|super)\)\s+const)\s+{re.escape(pattern)}\s*:\s*&str\s*=\s*""",
    )
    m = const_pat.search(source)
    if m:
        line_number = source[: m.start()].count("\n") + 1
        # Find the string literal after the '='
        after_eq = m.end()
        # Skip whitespace
        while after_eq < len(source) and source[after_eq] in (" ", "\t", "\n", "\r"):
            after_eq += 1
        # Check for include_str!
        if source[after_eq:].startswith("include_str!"):
            # Extract the file path from include_str!("...")
            inc_match = re.match(r'include_str!\(\s*"([^"]+)"\s*\)', source[after_eq:])
            if inc_match:
                import os
                inc_path = inc_match.group(1)
                base_dir = os.path.dirname(filepath)
                resolved = os.path.normpath(os.path.join(base_dir, inc_path))
                try:
                    with open(resolved, encoding="utf-8") as f:
                        return f.read(), line_number
                except OSError:
                    return None, line_number
            return None, line_number
        val = _find_string_at(source, after_eq)
        if val is not None:
            return val, line_number

    # Strategy 2: simple const with = "..." (no type annotation)
    simple_pat = re.compile(
        rf"""(?:const|static|let)\s+{re.escape(pattern)}\s*=\s*""",
    )
    m = simple_pat.search(source)
    if m:
        line_number = source[: m.start()].count("\n") + 1
        after_eq = m.end()
        while after_eq < len(source) and source[after_eq] in (" ", "\t", "\n", "\r"):
            after_eq += 1
        val = _find_string_at(source, after_eq)
        if val is not None:
            return val, line_number

    # Strategy 3: LazyLock<ToolSpec> or ToolSpec { ... } — find the pattern name, then locate `description:`
    # inside a ResponsesApiTool (or similar struct), skipping JsonSchema description fields.
    lazy_pat = re.compile(
        rf"""(?:pub(?:\([^)]*\))?\s+)?static\s+{re.escape(pattern)}\s*:\s*LazyLock<""",
    )
    m = lazy_pat.search(source)
    if m:
        line_number = source[: m.start()].count("\n") + 1
        # Find the block after this declaration
        block_start = source.find("{", m.end())
        if block_start == -1:
            return None, line_number
        # Look for ResponsesApiTool { ... description: ... } pattern
        desc_search_region = source[block_start : block_start + 15000]
        # Find ResponsesApiTool block, then its description field
        api_tool_match = re.search(r"ResponsesApiTool\s*\{", desc_search_region)
        if api_tool_match:
            api_start = block_start + api_tool_match.end()
            # Now find `description:` within this struct (first one after the name field)
            api_region = source[api_start : api_start + 10000]
            # Skip the `name:` field, then find `description:`
            name_match = re.search(r"name:\s*", api_region)
            if name_match:
                after_name = name_match.end()
            else:
                after_name = 0
            desc_match = re.search(r"description:\s*", api_region[after_name:])
            if desc_match:
                desc_pos = api_start + after_name + desc_match.end()
                while desc_pos < len(source) and source[desc_pos] in (" ", "\t", "\n", "\r"):
                    desc_pos += 1
                if source[desc_pos:].startswith("format!"):
                    fmt_start = source.find("(", desc_pos) + 1
                    while fmt_start < len(source) and source[fmt_start] in (" ", "\t", "\n", "\r"):
                        fmt_start += 1
                    val = _find_string_at(source, fmt_start)
                    if val is not None:
                        return val, line_number
                else:
                    val = _find_string_at(source, desc_pos)
                    if val is not None:
                        return val, line_number
        else:
            # Fallback: no ResponsesApiTool found, try first description: in the block
            desc_match = re.search(r"description:\s*", desc_search_region)
            if desc_match:
                desc_pos = block_start + desc_match.end()
                while desc_pos < len(source) and source[desc_pos] in (" ", "\t", "\n", "\r"):
                    desc_pos += 1
                if source[desc_pos:].startswith("format!"):
                    fmt_start = source.find("(", desc_pos) + 1
                    while fmt_start < len(source) and source[fmt_start] in (" ", "\t", "\n", "\r"):
                        fmt_start += 1
                    val = _find_string_at(source, fmt_start)
                    if val is not None:
                        return val, line_number
                else:
                    val = _find_string_at(source, desc_pos)
                    if val is not None:
                        return val, line_number

    # Strategy 4: Grep line by line for the pattern name
    for i, line in enumerate(lines, 1):
        if pattern in line:
            # Check if this line has a string after '='
            eq_idx = line.find("=")
            if eq_idx >= 0:
                after = line[eq_idx + 1 :].strip()
                val = _find_string_at(after, 0)
                if val is not None:
                    return val, i
            # Check for string on next lines
            if i < len(lines):
                next_line = lines[i].strip()  # lines[i] is line i+1 (0-indexed)
                val = _find_string_at(next_line, 0)
                if val is not None:
                    return val, i

    return None, 0


def extract_rust_function(filepath: str, pattern: str) -> tuple[str | None, int]:
    """Find `fn {pattern}` in a .rs file and extract the full function body.

    Uses brace-matching from `fn` to the closing `}`.
    Returns (source_code, line_number) or (None, 0) on failure.
    """
    try:
        with open(filepath, encoding="utf-8") as f:
            source = f.read()
    except OSError:
        return None, 0

    # Find `fn pattern(` or `fn pattern <` (for generic functions)
    fn_pat = re.compile(rf"\bfn\s+{re.escape(pattern)}\s*[<(]")
    m = fn_pat.search(source)
    if not m:
        return None, 0

    line_number = source[: m.start()].count("\n") + 1

    # Walk backwards to find any attributes, pub, async, etc. on lines before
    fn_start = m.start()
    # Walk backwards to include visibility, async, attributes
    line_start = source.rfind("\n", 0, fn_start)
    if line_start == -1:
        line_start = 0
    else:
        line_start += 1

    # Check for attributes and modifiers on preceding lines
    actual_start = line_start
    while actual_start > 0:
        prev_line_end = actual_start - 1
        prev_line_start = source.rfind("\n", 0, prev_line_end)
        if prev_line_start == -1:
            prev_line_start = 0
        else:
            prev_line_start += 1
        prev_line = source[prev_line_start:prev_line_end].strip()
        if prev_line.startswith("#[") or prev_line.startswith("///") or prev_line.startswith("//"):
            actual_start = prev_line_start
        elif prev_line.startswith("pub") or prev_line.startswith("async"):
            actual_start = prev_line_start
        else:
            break

    # Now find the opening brace
    brace_pos = source.find("{", m.end())
    if brace_pos == -1:
        return None, 0

    # Brace-match to find closing brace
    depth = 1
    i = brace_pos + 1
    in_string = False
    in_raw_string = False
    raw_hashes = 0
    in_line_comment = False
    in_block_comment = False

    while i < len(source) and depth > 0:
        ch = source[i]

        if in_line_comment:
            if ch == "\n":
                in_line_comment = False
            i += 1
            continue

        if in_block_comment:
            if ch == "*" and i + 1 < len(source) and source[i + 1] == "/":
                in_block_comment = False
                i += 2
                continue
            i += 1
            continue

        if in_raw_string:
            if ch == '"':
                # Check for closing hashes
                end_hashes = 0
                j = i + 1
                while j < len(source) and source[j] == "#" and end_hashes < raw_hashes:
                    end_hashes += 1
                    j += 1
                if end_hashes == raw_hashes:
                    in_raw_string = False
                    i = j
                    continue
            i += 1
            continue

        if in_string:
            if ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_string = False
            i += 1
            continue

        # Check for comments
        if ch == "/" and i + 1 < len(source):
            if source[i + 1] == "/":
                in_line_comment = True
                i += 2
                continue
            if source[i + 1] == "*":
                in_block_comment = True
                i += 2
                continue

        # Check for raw strings
        if ch == "r" and i + 1 < len(source):
            j = i + 1
            h = 0
            while j < len(source) and source[j] == "#":
                h += 1
                j += 1
            if j < len(source) and source[j] == '"' and h > 0:
                in_raw_string = True
                raw_hashes = h
                i = j + 1
                continue

        # Check for regular strings
        if ch == '"':
            in_string = True
            i += 1
            continue

        # Check for char literals
        if ch == "'":
            # Skip char literals like 'a', '\n', '\\'
            if i + 2 < len(source) and source[i + 1] == "\\" and i + 3 < len(source) and source[i + 3] == "'":
                i += 4
                continue
            if i + 2 < len(source) and source[i + 2] == "'":
                i += 3
                continue
            # Might be a lifetime, just advance
            i += 1
            continue

        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1

        i += 1

    if depth != 0:
        return None, 0

    # i now points one past the closing brace
    fn_source = source[actual_start:i]
    return fn_source, line_number
