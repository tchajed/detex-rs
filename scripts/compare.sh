#!/bin/bash

# Script to compare outputs of detex-rs and opendetex
# Usage: ./compare_detex.sh [arguments for detex]

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPENDETEX_DIR="$SCRIPT_DIR/../opendetex-2.8.11"
OPENDETEX_BIN="$OPENDETEX_DIR/detex"

# Build opendetex if it doesn't exist
if [ ! -f "$OPENDETEX_BIN" ]; then
  echo "Building opendetex..."
  (cd "$OPENDETEX_DIR" && make)
  echo ""
fi

# Create temporary files for outputs
DETEX_RS_OUT=$(mktemp -t detex-rs.XXXXXX)
OPENDETEX_OUT=$(mktemp -t opendetex.XXXXXX)

# Clean up temp files on exit
trap "rm -f $DETEX_RS_OUT $OPENDETEX_OUT" EXIT

# Run detex-rs
detex-rs "$@" >"$DETEX_RS_OUT" 2>&1

# Run opendetex
"$OPENDETEX_BIN" "$@" >"$OPENDETEX_OUT" 2>&1

# Show diff
diff -u "$OPENDETEX_OUT" "$DETEX_RS_OUT" || true
