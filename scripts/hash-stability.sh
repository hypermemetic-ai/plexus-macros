#!/usr/bin/env bash
# RETURNS-10 acceptance 2: method hashes are identical across clean builds.
#
# Runs the tests that pin real hashes — the conformance fixture's
# (`expected/schema.json` and every frame's `plexus_hash`, checked by
# `fixtures/conformance/tests/serve.rs`) and RETURNS-10's table
# (`tests/returns10_shape_hash_tests.rs`, `BASE_FETCH_HASH`) — in two separate
# cargo invocations, each a clean build in its own target directory:
#
#   1. default features;
#   2. with serde_json's `preserve_order` on, the flip any crate in a
#      backend's graph can cause by feature unification (RETURNS-24: it
#      reorders every schema's raw JSON keys; the canonical form must not see
#      it).
#
# A pin that holds in both builds is a hash that did not move between them.
#
#   scripts/hash-stability.sh            # target dirs under target/hash-stability
#   OUT=/tmp/hs scripts/hash-stability.sh
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
out=${OUT:-$here/target/hash-stability}
rm -rf "$out"
mkdir -p "$out"

run() { # label, extra cargo args…
  local label=$1; shift
  echo "== build $label: clean, CARGO_TARGET_DIR=$out/$label $*" >&2
  CARGO_TARGET_DIR="$out/$label" cargo test --manifest-path "$here/Cargo.toml" --quiet \
    -p plexus-macros --test returns10_shape_hash_tests \
    -p conformance --test serve "$@" 2>&1 | tee "$out/$label.log" | grep -E '^test result|FAILED|panicked' >&2
}

run default
run preserve-order --features serde_json/preserve_order

for label in default preserve-order; do
  if grep -qE 'FAILED|panicked' "$out/$label.log" || ! grep -q '^test result: ok' "$out/$label.log"; then
    echo "hash-stability: build $label failed (see $out/$label.log)" >&2
    exit 1
  fi
done
echo "hash-stability: the pinned hashes held in both clean builds" >&2
