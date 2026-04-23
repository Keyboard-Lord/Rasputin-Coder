#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
