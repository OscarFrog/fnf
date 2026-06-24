#!/usr/bin/env bash
# Produce the two source tarballs required by fnf.spec:
#   Source0: fnf-<version>.tar.gz      (GitHub archive / git export)
#   Source1: fnf-<version>-vendor.tar.gz (cargo vendor snapshot)
#
# Run from the repo root.  Requires: git, cargo, tar.

set -euo pipefail

VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
ARCHIVE="fnf-${VERSION}"

echo "==> Version: ${VERSION}"

# Source0 — git archive when available, tar fallback for CI containers
echo "==> Creating ${ARCHIVE}.tar.gz ..."
if git rev-parse HEAD > /dev/null 2>&1; then
    git archive --prefix="${ARCHIVE}/" HEAD | gzip -n > "pkg/${ARCHIVE}.tar.gz"
else
    tar czf "pkg/${ARCHIVE}.tar.gz" \
        --transform "s|^\./|${ARCHIVE}/|" \
        --exclude='./.git' --exclude='./pkg' --exclude='./target' \
        .
fi

# Source1 — vendor snapshot
echo "==> Creating ${ARCHIVE}-vendor.tar.gz ..."
cargo vendor --quiet vendor
tar czf "pkg/${ARCHIVE}-vendor.tar.gz" vendor
rm -rf vendor

echo "==> Done. Files in pkg/:"
ls -lh "pkg/${ARCHIVE}.tar.gz" "pkg/${ARCHIVE}-vendor.tar.gz"
