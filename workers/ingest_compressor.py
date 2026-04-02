#!/usr/bin/env python3
"""Cortex ingest compressor -- preprocesses text before cortex_store.

Strips filler, normalizes paths, deduplicates against existing memories,
and compresses to dense factual statements.

Usage:
    # As a library (called from Claude Code hooks or scripts)
    from ingest_compressor import compress_for_cortex, check_duplicate

    compressed = compress_for_cortex(raw_text)
    is_dup = check_duplicate(compressed, cortex_url="http://127.0.0.1:7437")

    # As CLI
    python ingest_compressor.py "raw text to compress"
    echo "raw text" | python ingest_compressor.py --stdin
    python ingest_compressor.py --cleanup  # deduplicate existing decisions
"""

import json
import re
import sys
import urllib.request
import urllib.error
from pathlib import Path

CORTEX_URL = "http://127.0.0.1:7437"
HOME = str(Path.home())


def _get_auth_token() -> str | None:
    """Read Cortex auth token from ~/.cortex/cortex.token."""
    token_path = Path.home() / ".cortex" / "cortex.token"
    try:
        return token_path.read_text().strip()
    except FileNotFoundError:
        return None

# Filler patterns to strip (model narration, boilerplate)
FILLER_PATTERNS = [
    r"^I will now\b.*$",
    r"^I am going to\b.*$",
    r"^Let me (first |now |also )(check|read|look|verify)\b.*$",
    r"^Now I will\b.*$",
    r"^Next,? I (will|need to|should)\b.*$",
    r"^First,? I (will|need to|should)\b.*$",
    r"^After completing (this|the|all)\b.*$",
    r"^The (current |following )?task (is|was)\b.*$",
    r"^Summary:?\s*$",
    r"^Brain: ONLINE\b.*$",
    r"^Stored to Cortex:.*$",
    r"^\s*$",
]

# Path normalization rules
PATH_REPLACEMENTS = [
    (re.compile(re.escape(HOME.replace("\\", "/"))), "~"),
    (re.compile(re.escape(HOME)), "~"),
    (re.compile(r"C:\\Users\\[^\\]+"), "~"),
    (re.compile(r"C:/Users/[^/]+"), "~"),
    (re.compile(r"/c/Users/[^/]+"), "~"),
]


def strip_filler(text: str) -> str:
    """Remove model narration and boilerplate lines.

    Only strips lines that are PURELY filler (no facts after the filler prefix).
    A line like "Successfully completed. Three bugs found." is kept because
    it contains a fact after the filler.
    """
    lines = text.split("\n")
    kept = []
    for line in lines:
        stripped = line.strip()
        # Only strip if the ENTIRE line matches a filler pattern
        is_filler = False
        for p in FILLER_PATTERNS:
            m = re.match(p, stripped, re.IGNORECASE)
            if m and m.end() >= len(stripped) - 1:
                is_filler = True
                break
        if not is_filler:
            kept.append(line)
    return "\n".join(kept).strip()


def normalize_paths(text: str) -> str:
    """Replace full home paths with ~."""
    for pattern, replacement in PATH_REPLACEMENTS:
        text = pattern.sub(replacement, text)
    return text


def compress_whitespace(text: str) -> str:
    """Collapse multiple blank lines, trim trailing whitespace."""
    text = re.sub(r"\n{3,}", "\n\n", text)
    lines = [line.rstrip() for line in text.split("\n")]
    return "\n".join(lines).strip()


def extract_key_facts(text: str) -> str:
    """If text is very long, extract the first sentence of each paragraph."""
    if len(text) < 500:
        return text

    paragraphs = text.split("\n\n")
    facts = []
    for para in paragraphs:
        para = para.strip()
        if not para:
            continue
        # Take first sentence (up to first period followed by space or end)
        match = re.match(r"^(.+?\.)\s", para)
        if match:
            facts.append(match.group(1))
        else:
            # No sentence boundary -- take first 200 chars
            facts.append(para[:200])
    return " ".join(facts)


def compress_for_cortex(text: str, max_length: int = 500) -> str:
    """Full compression pipeline: strip filler, normalize paths, compress.

    Args:
        text: Raw text to compress
        max_length: Target maximum length (soft limit)

    Returns:
        Compressed text suitable for cortex_store
    """
    result = strip_filler(text)
    result = normalize_paths(result)
    result = compress_whitespace(result)

    # If still too long, extract key facts
    if len(result) > max_length:
        result = extract_key_facts(result)

    # Final trim
    if len(result) > max_length:
        result = result[:max_length].rsplit(" ", 1)[0] + "..."

    return result


def check_duplicate(
    text: str,
    cortex_url: str = CORTEX_URL,
    threshold: float = 0.85,
) -> dict | None:
    """Check if a similar memory already exists in Cortex.

    Returns the matching entry if duplicate found, None otherwise.
    """
    try:
        # Use peek (lightweight) to check for similar content
        token = _get_auth_token()
        url = f"{cortex_url}/recall?q={urllib.parse.quote(text[:200])}&budget=200"
        req = urllib.request.Request(url)
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        with urllib.request.urlopen(req, timeout=3) as resp:
            data = json.loads(resp.read().decode())

        results = data.get("results", [])
        for r in results:
            relevance = r.get("relevance", 0)
            if relevance >= threshold:
                return r

    except (urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        pass

    return None


def cleanup_existing(
    cortex_url: str = CORTEX_URL,
    dry_run: bool = True,
) -> list[dict]:
    """Scan existing decisions for duplicates and noise.

    Returns list of entries that should be removed or compressed.
    """
    import urllib.parse

    issues = []

    try:
        # Fetch all decisions
        token = _get_auth_token()
        url = f"{cortex_url}/recall?q=*&budget=2000"
        req = urllib.request.Request(url)
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())

        results = data.get("results", [])

        seen_sources = {}
        for r in results:
            source = r.get("source", "")
            excerpt = r.get("excerpt", "")

            # Check for duplicates (same source)
            if source in seen_sources:
                issues.append({
                    "type": "duplicate",
                    "source": source,
                    "excerpt": excerpt[:100],
                    "original": seen_sources[source][:100],
                })
            else:
                seen_sources[source] = excerpt

            # Check for filler content
            compressed = compress_for_cortex(excerpt)
            if len(compressed) < len(excerpt) * 0.5:
                issues.append({
                    "type": "verbose",
                    "source": source,
                    "original_len": len(excerpt),
                    "compressed_len": len(compressed),
                    "savings": f"{(1 - len(compressed) / len(excerpt)) * 100:.0f}%",
                    "compressed": compressed[:200],
                })

    except (urllib.error.URLError, json.JSONDecodeError, TimeoutError) as e:
        issues.append({"type": "error", "message": str(e)})

    return issues


if __name__ == "__main__":
    import urllib.parse

    args = sys.argv[1:]

    if "--cleanup" in args:
        dry_run = "--apply" not in args
        print(f"Scanning Cortex for issues ({'DRY RUN' if dry_run else 'APPLYING FIXES'})...")
        issues = cleanup_existing(dry_run=dry_run)
        for issue in issues:
            if issue["type"] == "duplicate":
                print(f"  DUPLICATE: {issue['source']}")
            elif issue["type"] == "verbose":
                print(f"  VERBOSE ({issue['savings']} compressible): {issue['source']}")
                print(f"    compressed: {issue['compressed']}")
            elif issue["type"] == "error":
                print(f"  ERROR: {issue['message']}")
        print(f"\nTotal issues: {len(issues)}")

    elif "--stdin" in args:
        text = sys.stdin.read()
        print(compress_for_cortex(text))

    elif args:
        text = " ".join(args)
        print(compress_for_cortex(text))

    else:
        print("Usage:")
        print('  python ingest_compressor.py "text to compress"')
        print("  echo text | python ingest_compressor.py --stdin")
        print("  python ingest_compressor.py --cleanup [--apply]")
