#!/usr/bin/env bash
set -euxo pipefail

cd "$(dirname "$0")"
cargo clean
cargo build "$@"
