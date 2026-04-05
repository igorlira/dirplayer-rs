#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../vm-rust"
SNAPSHOT_UPDATE=1 cargo test --test mod e2e -- "$@"
echo "Snapshots updated in tests/snapshots/reference/"
