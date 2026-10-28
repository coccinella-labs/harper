#!/usr/bin/env python3

import argparse
import pathlib
import re
import sys


def replace_once(content: str, pattern: str, replacement: str) -> str:
    updated, count = re.subn(pattern, replacement, content, count=1, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"pattern not found exactly once: {pattern}")
    return updated


def replace_exact(content: str, pattern: str, replacement: str, expected: int) -> str:
    updated, count = re.subn(pattern, replacement, content, flags=re.MULTILINE)
    if count != expected:
        raise ValueError(f"pattern found {count} times, expected {expected}: {pattern}")
    return updated


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Update harper-ai Homebrew formula to a released Harper version."
    )
    parser.add_argument("--release-tag", required=True, help="Release tag like harper-0.17.1")
    parser.add_argument("--sha256", required=True, help="SHA256 of the release source tarball")
    parser.add_argument("--formula-path", required=True, help="Path to Formula/harper-ai.rb")
    args = parser.parse_args()

    release_tag = args.release_tag
    match = re.match(r'^harper-(?:workspace-)?(.+)$', release_tag)
    if not match:
        raise SystemExit(f"release tag must start with 'harper-' or 'harper-workspace-': {release_tag}")
    version = match.group(1)
    tarball_url = (
        f"https://github.com/coccinella-labs/harper/archive/refs/tags/{release_tag}.tar.gz"
    )

    formula_path = pathlib.Path(args.formula_path)
    content = formula_path.read_text(encoding="utf-8")

    if (
        f'version "{version}"' in content
        and content.count(f'      url "{tarball_url}"') == 2
        and content.count(f'      sha256 "{args.sha256}"') == 2
    ):
        print("formula already matches requested release")
        return 0

    content = replace_once(content, r'^  version ".*"$', f'  version "{version}"')
    content = replace_exact(
        content,
        r'^      url "https://github\.com/coccinella-labs/harper/archive/refs/tags/.*\.tar\.gz"$',
        f'      url "{tarball_url}"',
        2,
    )
    content = replace_exact(
        content,
        r'^      sha256 "[0-9a-f]{64}"$',
        f'      sha256 "{args.sha256}"',
        2,
    )

    formula_path.write_text(content, encoding="utf-8")
    print(f"updated {formula_path} to {release_tag}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as err:
        print(err, file=sys.stderr)
        raise SystemExit(1)
