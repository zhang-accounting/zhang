#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

# Print only missing variable names, never credential values.
for name in ZHANG_S3_BUCKET ZHANG_S3_ENDPOINT ZHANG_S3_REGION ZHANG_S3_ACCESS_KEY_ID ZHANG_S3_SECRET_ACCESS_KEY; do
  if [[ -z "${!name:-}" ]]; then
    printf 'Missing required environment variable: %s\n' "$name" >&2
    exit 1
  fi
done

exec ./target/release/zhang serve . \
  --source s3 \
  --addr 0.0.0.0 \
  --port "${PORT:-8000}" \
  --endpoint "${ZHANG_DEMO_ENDPOINT:-main.zhang}" \
  --no-report
