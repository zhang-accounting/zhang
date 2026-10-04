#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

# The lockfile uses pnpm 9. Build the UI before Rust embeds frontend/dist.
npx --yes pnpm@9.15.9 --dir frontend install --frozen-lockfile
npx --yes pnpm@9.15.9 --dir frontend build
cargo build --release --locked -p zhang --features zhang-server/frontend --jobs 2
