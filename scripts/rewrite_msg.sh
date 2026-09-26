# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright 2026 coccinella-labs

#!/bin/bash

# Rewrite commit messages: lowercase first line and truncate to 60 chars, or set default if empty

FIRST_LINE=true
HAS_LINES=false
while IFS= read -r line; do
    HAS_LINES=true
    if $FIRST_LINE; then
        # Lowercase and truncate first line to 60 chars, or set default if empty
        if [ -n "$line" ]; then
            line=$(echo "$line" | tr '[:upper:]' '[:lower:]' | cut -c1-60)
        else
            line="no commit message"
        fi
        FIRST_LINE=false
    fi
    echo "$line"
done

# If no lines at all, output default message
if ! $HAS_LINES; then
    echo "no commit message"
fi
