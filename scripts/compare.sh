#!/bin/bash

set -eu

# Script to compare outputs of detex-rs and opendetex
# Usage: ./compare_detex.sh [arguments for detex]

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DETEX_RS_BIN="$SCRIPT_DIR/../target/debug/detex-rs"
OPENDETEX_DIR="$SCRIPT_DIR/../opendetex-2.8.11"
OPENDETEX_BIN="$OPENDETEX_DIR/detex"

# Build opendetex if it doesn't exist
if [ ! -f "$OPENDETEX_BIN" ]; then
  (cd "$OPENDETEX_DIR" && make)
fi

cargo build --manifest-path "$SCRIPT_DIR/../Cargo.toml" --quiet

# Create temporary files for outputs
DETEX_RS_OUT=$(mktemp -t detex-rs-stdout.XXXXXX)
DETEX_RS_ERR=$(mktemp -t detex-rs-stderr.XXXXXX)
OPENDETEX_OUT=$(mktemp -t opendetex-stdout.XXXXXX)
OPENDETEX_ERR=$(mktemp -t opendetex-stderr.XXXXXX)

# Clean up temp files on exit
trap "rm -f $DETEX_RS_OUT $DETEX_RS_ERR $OPENDETEX_OUT $OPENDETEX_ERR" EXIT

# Run detex-rs
"$DETEX_RS_BIN" "$@" >"$DETEX_RS_OUT" 2>"$DETEX_RS_ERR"

# Run opendetex
"$OPENDETEX_BIN" "$@" >"$OPENDETEX_OUT" 2>"$OPENDETEX_ERR"

# Compare stdout
if ! diff -u "$OPENDETEX_OUT" "$DETEX_RS_OUT"; then
  echo "stdout differs!"
  exit 1
fi

# Compare stderr
if ! diff -u "$OPENDETEX_ERR" "$DETEX_RS_ERR"; then
  echo "stderr differs!"
  exit 1
fi
