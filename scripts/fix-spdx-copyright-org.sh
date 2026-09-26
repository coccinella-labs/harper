#!/bin/bash
# Fix stale org name in SPDX/copyright headers.
# Stale:   harpertoken       (e.g. "// Copyright 2026 harpertoken")
# Correct: coccinella-labs   (per git remote origin, COMMERCIAL_LICENSE,
#                             and github.repository_owner checks)
# Only touches lines containing "Copyright" so repo URLs like
# github.com/coccinella-labs/harper are left alone.
# Package and publisher identities (npm scope, VS Code publisher, MSIX) are
# not header text and are deliberately out of scope here.
set -euo pipefail

STALE="harpertoken"
CORRECT="coccinella-labs"

cd "$(dirname "$0")/.."

# Find text files containing a stale Copyright header, excluding
# version-control, build, and dependency dirs.
matches=$(grep -rlI "Copyright.*${STALE}" \
  --exclude=fix-spdx-copyright-org.sh \
  --exclude-dir=.git \
  --exclude-dir=target \
  --exclude-dir=node_modules \
  --exclude-dir=.venv \
  --exclude-dir=__pycache__ \
  . 2>/dev/null || true)

if [ -z "$matches" ]; then
  echo "No stale Copyright headers (${STALE}) found."
  exit 0
fi

count=$(echo "$matches" | wc -l | tr -d ' ')
echo "Found ${count} file(s) with stale Copyright header '${STALE}':"

updated=0
while IFS= read -r f; do
  echo "  ${f}"
  # Replace stale org with correct org ONLY on Copyright lines, preserving
  # the year. Uses $STALE/$CORRECT so the two halves cannot drift apart
  # and silently turn this into a no-op.
  # perl -pi is portable across macOS/BSD and GNU/Linux (unlike sed -i).
  perl -pi -e "s/${STALE}/${CORRECT}/g if /Copyright/" "$f"
  updated=$((updated + 1))
done <<< "$matches"

echo ""
echo "Updated ${updated} file(s): Copyright ... ${STALE} -> Copyright ... ${CORRECT}"

# Verification
remaining=$(grep -rlI "Copyright.*${STALE}" \
  --exclude=fix-spdx-copyright-org.sh \
  --exclude-dir=.git \
  --exclude-dir=target \
  --exclude-dir=node_modules \
  --exclude-dir=.venv \
  --exclude-dir=__pycache__ \
  . 2>/dev/null || true)
if [ -n "$remaining" ]; then
  echo "WARNING: stale headers remain in:"
  echo "$remaining"
  exit 1
fi
echo "Verified: no stale Copyright headers remain."
