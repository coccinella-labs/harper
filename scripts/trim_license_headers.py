#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright 2026 coccinella-labs
#
# Script: trim_license_headers.py
# Trims the verbose Apache License boilerplate from a leading file header,
# leaving an SPDX identifier plus a copyright line. The copyright year is
# read from the file and re-emitted verbatim, so re-running never rewrites it.
#
# Usage:
#   python3 scripts/trim_license_headers.py --check   report, write nothing
#   python3 scripts/trim_license_headers.py           apply the trim

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).parent.parent

SPDX_ID = "MIT OR Apache-2.0"
TERMINATOR = "limitations under the License."

# Never rewritten. These can match the terminator but are not file headers.
EXCLUDE = {
    "LICENSE-APACHE",  # is the Apache License text itself
    "LICENSE",  # license text
    "COMMERCIAL_LICENSE",  # license text
}

COPYRIGHT_RE = r"Copyright (?P<year>\d{4}) coccinella-labs"
SPDX_RE = r"SPDX-License-Identifier:"


def tracked_targets():
    """Tracked files containing the Apache terminator, minus exclusions."""
    out = subprocess.run(
        ["git", "grep", "-l", TERMINATOR],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    return [f for f in sorted(out.stdout.split()) if f not in EXCLUDE]


def style_of(line):
    """Return the comment token for a header line, or None if not a comment."""
    m = re.match(r"^(//|#)(?=\s|$)", line)
    return m.group(1) if m else None


def is_blank_comment(line, style):
    return re.match(rf"^{re.escape(style)}\s*$", line) is not None


def find_header_start(lines):
    """Index and style of the 'Licensed under ...' line that opens the block."""
    for i in range(min(len(lines), 40)):
        for style in ("#", "//"):
            if re.match(rf"^{re.escape(style)}\s*Licensed under\b", lines[i]):
                return i, style
    return None, None


def find_terminator_end(lines, style):
    """Index just past the 'limitations under the License.' line."""
    pattern = re.compile(rf"^{re.escape(style)}\s*{re.escape(TERMINATOR)}")
    for i in range(len(lines)):
        if pattern.match(lines[i]):
            return i + 1
    return None


def scan_back(lines, start, style, matcher):
    """Walk upwards over blank comment lines looking for a match."""
    for i in range(start - 1, -1, -1):
        if matcher(lines[i]):
            return i
        if lines[i].strip() and not is_blank_comment(lines[i], style):
            return None
    return None


def trim_hashed(text):
    """Trim a // or # style header. Returns new text, or None if not a header."""
    lines = text.splitlines(keepends=True)
    if not lines:
        return None

    # Derive the comment style from the block itself. The header is not
    # always the first thing in a file (see .yamllint.yml).
    start, style = find_header_start(lines)
    if start is None:
        return None
    end = find_terminator_end(lines, style)
    if end is None:
        return None

    cr_re = re.compile(rf"^{re.escape(style)}\s*{COPYRIGHT_RE}")
    cr_idx = scan_back(lines, start, style, lambda l: bool(cr_re.match(l)))
    if cr_idx is None:
        return None
    year = cr_re.match(lines[cr_idx]).group("year")

    spdx_re = re.compile(rf"^{re.escape(style)}\s*{SPDX_RE}")
    spdx_idx = scan_back(lines, cr_idx, style, lambda l: bool(spdx_re.match(l)))

    # Keep every line above the header block verbatim. The block is not
    # always at the top of the file: it can follow a shebang, or real
    # config such as `extends:` in .yamllint.yml.
    keep_to = spdx_idx if spdx_idx is not None else cr_idx
    prefix = lines[:keep_to]

    header = [
        f"{style} SPDX-License-Identifier: {SPDX_ID}\n",
        f"{style} Copyright {year} coccinella-labs\n",
        "\n",
    ]

    tail = lines[end:]
    if tail and not tail[0].strip():
        tail = tail[1:]  # drop the single blank line the block left behind

    return "".join(prefix + header + tail)


HTML_APACHE_BLOCK = re.compile(
    r"<!--\s*\nCopyright (?P<year>\d{4}) coccinella-labs\s*\n.*?" + re.escape(TERMINATOR) + r".*?-->\s*\n+",
    re.DOTALL,
)


def trim_html(text):
    """Trim an HTML-comment header (Markdown). Returns new text, or None."""
    if not HTML_APACHE_BLOCK.search(text):
        return None
    return HTML_APACHE_BLOCK.sub(
        "<!--\nSPDX-License-Identifier: {}\nCopyright \\g<year> coccinella-labs\n-->\n\n".format(SPDX_ID),
        text,
        count=1,
    )


def trim(path):
    original = path.read_text(encoding="utf-8")
    updated = trim_html(original)
    if updated is None:
        updated = trim_hashed(original)
    if updated is None or updated == original:
        return None
    return updated


def main():
    check_only = "--check" in sys.argv[1:]
    targets = tracked_targets()
    changed = []

    for rel in targets:
        path = REPO / rel
        updated = trim(path)
        if updated is None:
            continue
        changed.append(rel)
        if not check_only:
            path.write_text(updated, encoding="utf-8")

    verb = "Would trim" if check_only else "Trimmed"
    print(f"\n{verb} {len(changed)} of {len(targets)} candidate file(s):")
    for rel in changed:
        print(f"  {rel}")
    if check_only and changed:
        print("\nRe-run without --check to apply.")


if __name__ == "__main__":
    main()
