#!/usr/bin/env python3
# Keep website/nav-updates.json in sync with the harper-update-version
# meta stamps in each website page. The stamps are maintained by hand in
# the HTML; this script only mirrors them into the nav update manifest.
#
#   python3 scripts/sync-nav-updates.py          # update in place
#   python3 scripts/sync-nav-updates.py --check  # exit 1 if out of sync

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WEBSITE = ROOT / "website"
MANIFEST = WEBSITE / "nav-updates.json"

META_RE = re.compile(r'harper-update-version"\s+content="([^"]+)"')


def page_stamps():
    stamps = {}
    for html in sorted(WEBSITE.glob("*.html")):
        match = META_RE.search(html.read_text())
        if match:
            stamps[html.name] = match.group(1)
    return stamps


def main():
    check_only = "--check" in sys.argv
    current = page_stamps()
    loaded = json.loads(MANIFEST.read_text())

    outdated = {name for name, stamp in current.items() if loaded.get(name) != stamp}
    extra = set(loaded) - set(current)

    if not outdated and not extra:
        print("nav-updates.json is in sync")
        return 0

    print("nav-updates.json is out of sync:")
    for name in sorted(outdated):
        print(f"  {name}: {loaded.get(name)!r} -> {current[name]!r}")
    for name in sorted(extra):
        print(f"  {name}: stale entry, page no longer exists")

    if check_only:
        return 1

    loaded.update(current)
    for name in extra:
        del loaded[name]

    MANIFEST.write_text(json.dumps(loaded, indent=2) + "\n")
    print("Updated website/nav-updates.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())