#!/usr/bin/env bash
# verify-published.sh — post-publish verification harness (D-05, HUB-01/HUB-05).
#
# Proves "published" in the strict sense: a clean consumer can resolve the crate
# `mcp-server-hub` from crates.io / its GitHub Release assets and end up with a working
# `mcp-hub` binary — exercising BOTH install paths:
#
#   1. `cargo install mcp-server-hub`   — source install from crates.io (HUB-01: crate
#       name ≠ binary name; the installed binary is `mcp-hub`).
#   2. `cargo binstall mcp-server-hub`  — prebuilt-asset install, run with the STRICT
#       no-fallback flag so a missing / mis-named GitHub Release asset HARD-FAILS instead
#       of silently compiling from source (Pitfall 1 / HUB-04 / HUB-05).
#
# The strict flag is the primary guard. As DEFENSE-IN-DEPTH the binstall output is also
# captured and grepped for source-compile signals: if binstall ever compiles (even if it
# returned exit 0), this script exits non-zero. A fallback can NEVER silently pass.
#
# Run AFTER the v<version> tag publishes the crate + assets (Plan 04 runs this).
# crates.io propagation is seconds/minutes, so no long propagation poll is needed.
#
# Usage: bash scripts/verify-published.sh [version]   (version defaults to 0.1.1)
set -euo pipefail

VERSION="${1:-0.1.1}"
CRATE="mcp-server-hub"
BIN="mcp-hub"

# Strict no-fallback strategies (03-TOOLING-PINS.md §2, empirically confirmed 2026-06-22):
# disabling BOTH `compile` (cargo-install source fallback) AND `quick-install`
# (third-party cargo-quickinstall source) forces binstall to resolve ONLY from our
# GitHub Release assets (the `crate-meta-data` strategy). A missing asset → non-zero exit.
STRICT_STRATEGIES="compile,quick-install"

echo "==> Verifying ${CRATE} ${VERSION} is clean-installable from crates.io"

# --- Ensure cargo-binstall is available (first-party tool; install via cargo if absent) ---
if ! command -v cargo-binstall >/dev/null 2>&1; then
    echo "==> cargo-binstall not found — installing it (cargo install cargo-binstall --locked)"
    cargo install cargo-binstall --locked
fi

# --- Path 1: source install via `cargo install` ---
echo "==> [1/2] cargo install ${CRATE}@${VERSION} (source install)"
cargo install "${CRATE}" --version "${VERSION}" --locked --force
echo "==> Asserting '${BIN} --version' after cargo install"
"${BIN}" --version

# --- Path 2: prebuilt-asset install via strict `cargo binstall` ---
echo "==> [2/2] cargo binstall ${CRATE}@${VERSION} (strict: --disable-strategies ${STRICT_STRATEGIES})"
BINSTALL_LOG="$(mktemp -t mcp-hub-binstall.XXXXXX)"
trap 'rm -f "${BINSTALL_LOG}"' EXIT

# Capture output AND propagate binstall's own exit status (the strict flag makes a
# missing asset a non-zero exit). `set -o pipefail` ensures the cargo binstall exit
# status (not tee's) drives the pipeline result.
cargo binstall "${CRATE}" --version "${VERSION}" --no-confirm \
    --disable-strategies "${STRICT_STRATEGIES}" 2>&1 | tee "${BINSTALL_LOG}"

# DEFENSE-IN-DEPTH: fail if binstall ever source-compiled, even on a 0 exit.
if grep -Eiq 'compiling |building |installing from source|fall(ing)? *back|from source' "${BINSTALL_LOG}"; then
    echo "ERROR: cargo binstall fell back to a SOURCE COMPILE — prebuilt asset for ${CRATE} ${VERSION} is missing or mis-named." >&2
    echo "       This violates HUB-04/HUB-05 (Pitfall 1). The GitHub Release asset name must match" >&2
    echo "       the [package.metadata.binstall] pkg-url template byte-for-byte." >&2
    exit 1
fi

echo "==> Asserting '${BIN} --version' after cargo binstall"
"${BIN}" --version

echo
echo "VERIFIED: ${CRATE} ${VERSION} installs a working '${BIN}' binary via BOTH cargo install"
echo "          and strict (no-fallback) cargo binstall — no source-compile fallback occurred."
echo
echo "REMINDER (HUB-03): confirm the docs.rs build is green:"
echo "          https://docs.rs/crate/${CRATE}/${VERSION}/builds"
