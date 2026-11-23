#!/usr/bin/env bash
# Smoke test of a zhang image: serve a small ledger that declares a plugin in a container limited to 512 MiB, check
# that the server answers, that the official budget plugin and the declared plugin run, that the ledger has no errors,
# and that the container stops on SIGTERM within a few seconds.
#
#   .github/scripts/docker-smoke.sh <image>
#
# Environment:
#   CONTAINER_ENGINE  docker (default) or podman
#   SMOKE_PORT        host port to publish the server on (default 18765)
#   SMOKE_MEMORY      memory limit of the container (default 512m)
#   SMOKE_STOP_LIMIT  most seconds `stop` may take (default 5)
#   SMOKE_RUN_ARGS    more arguments for `run`, split on spaces (such as `--cgroup-parent …` for podman in a pod)
#
# Every check prints a PASS or FAIL line; the script exits non-zero when one failed and prints the container's logs.
set -euo pipefail

image=${1:?usage: docker-smoke.sh <image>}
engine=${CONTAINER_ENGINE:-docker}
port=${SMOKE_PORT:-18765}
memory=${SMOKE_MEMORY:-512m}
stop_limit=${SMOKE_STOP_LIMIT:-5}
read -r -a run_args <<<"${SMOKE_RUN_ARGS:-}"
base="http://127.0.0.1:${port}"
name="zhang-smoke-$$"
failures=0

pass() { echo "PASS: $*"; }
fail() {
  echo "FAIL: $*"
  failures=$((failures + 1))
}

ledger=$(mktemp -d)
cleanup() {
  "$engine" rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$ledger"
}
trap cleanup EXIT

# the logs and the end of the run: the container's logs on any failure, and the exit code
finish() {
  if [ "$failures" -gt 0 ]; then
    echo "::group::container logs"
    "$engine" logs "$name" 2>&1 || true
    echo "::endgroup::"
    echo "$failures check(s) failed"
    exit 1
  fi
  echo "all checks passed"
}

# A ledger with an operating currency, a second currency and its price, a budget with spending in both currencies,
# and a declared plugin: `zhang:mcp`, the Lua router plugin bundled in the binary, so that nothing is fetched and the
# plugin runtime is compiled (into the image's plugin cache, /root/.cache/zhang) as it is for any ledger plugin.
cat >"$ledger/main.zhang" <<'EOF'
option "title" "Docker smoke test"
option "operating_currency" "CNY"
option "features.plugin" "true"

plugin "zhang:mcp"

1970-01-01 commodity CNY
1970-01-01 commodity USD

2024-01-01 open Assets:Bank
2024-01-01 open Assets:Cash
2024-01-01 open Income:Salary
2024-01-01 open Expenses:Food
  budget: "Food"

2024-01-01 price USD 7 CNY

2024-01-01 budget Food CNY
2024-01-01 budget-add Food 1000 CNY

2024-01-05 * "Employer" "Salary"
  Assets:Bank 10000 CNY
  Income:Salary

2024-01-10 * "Canteen" "Lunch"
  Expenses:Food 100 CNY
  Assets:Bank

2024-01-12 * "Cafe" "Lunch abroad"
  Expenses:Food 10 USD @ 7 CNY
  Assets:Bank

2024-01-20 * "Market" "Groceries"
  Expenses:Food 50 CNY
  Assets:Cash
EOF
# the image runs as root and the ledger directory must be readable from inside the container
chmod 755 "$ledger"
chmod 644 "$ledger/main.zhang"

echo "starting $image with $engine, --memory $memory, on port $port"
"$engine" run -d --name "$name" "${run_args[@]}" --memory "$memory" -p "127.0.0.1:${port}:8000" -v "$ledger:/data" "$image" >/dev/null

# What a container that is not running anymore died of, with the plugin compile-memory hint for an OOM kill
report_dead() {
  local oom code
  oom=$("$engine" inspect --format '{{.State.OOMKilled}}' "$name" 2>/dev/null || echo unknown)
  code=$("$engine" inspect --format '{{.State.ExitCode}}' "$name" 2>/dev/null || echo unknown)
  if [ "$oom" = "true" ] || [ "$code" = "137" ]; then
    fail "memory: the container was killed (OOMKilled=$oom, exit $code) under --memory $memory. Expected until the fix of the plugin compile memory lands: compiling the plugin runtime needed more than the limit"
  else
    fail "the container stopped on its own (OOMKilled=$oom, exit $code)"
  fi
}

# The peak memory of the container's cgroup (cgroup v2, docker or podman, rootful), when it can be read
peak_memory() {
  local id dir
  id=$("$engine" inspect --format '{{.Id}}' "$name" 2>/dev/null) || return 0
  dir=$(find /sys/fs/cgroup -type d -name "*${id}*" 2>/dev/null | head -n 1)
  if [ -n "$dir" ] && [ -r "$dir/memory.peak" ]; then
    echo "peak memory of the container: $(($(cat "$dir/memory.peak") / 1024 / 1024)) MiB (limit $memory)"
  fi
}

# readiness: the server binds its port once the ledger and its plugins are loaded
ready=0
for _ in $(seq 1 120); do
  if [ "$("$engine" inspect --format '{{.State.Running}}' "$name")" != "true" ]; then
    break
  fi
  if curl -fsS -o /dev/null "$base/api/info" 2>/dev/null; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" != 1 ]; then
  if [ "$("$engine" inspect --format '{{.State.Running}}' "$name")" != "true" ]; then
    report_dead
  else
    fail "the server did not answer on $base within 120 s"
  fi
  finish
fi
pass "the server answers on $base"

# POST a built-in query by name; prints the JSON answer, fails the check on an HTTP error
builtin() {
  curl -fsS -X POST "$base/api/query/builtins/$1" -H 'Content-Type: application/json' -d "$2"
}

# GET /api/info: the version the image reports
if info=$(curl -fsS "$base/api/info") && version=$(jq -er '.data.version | select(length > 0)' <<<"$info"); then
  pass "GET /api/info answers, version $version, title $(jq -r '.data.title' <<<"$info")"
else
  fail "GET /api/info: ${info:-no answer}"
fi

# the declared plugin is loaded
if plugins=$(curl -fsS "$base/api/plugins") && jq -e '.data | map(.name) | index("zhang-mcp")' <<<"$plugins" >/dev/null; then
  pass "the declared plugin runs: $(jq -c '.data | map(.name)' <<<"$plugins")"
else
  fail "the declared plugin zhang:mcp is not in GET /api/plugins: ${plugins:-no answer}"
fi

# budgets.month, the official budget plugin: one row, Food in CNY, 1000 CNY assigned, 150 CNY and 10 USD at 7 CNY
# spent (220 CNY), 780 CNY available
if month=$(builtin budgets.month '{"params": {"month": "2024-01-01"}}') &&
  row=$(jq -ec '.data as $d | [$d.columns[].name] as $n | [$d.rows[] | [$n, .] | transpose | map({(.[0]): .[1]}) | add] | select(length == 1) | .[0]' <<<"$month") &&
  jq -e '.name == "Food" and .currency == "CNY" and (.activity | tonumber) == 220 and (.available.number | tonumber) == 780' <<<"$row" >/dev/null; then
  pass "budgets.month answers: $row"
else
  fail "budgets.month did not answer one Food row in CNY with 220 spent and 780 available: ${month:-no answer}"
fi

# the error list: none expected
if errors=$(builtin journals.errors '{"params": {"size": 100, "offset": 0}, "count_total": true}') &&
  count=$(jq -er '.data.total' <<<"$errors"); then
  if [ "$count" = 0 ]; then
    pass "the error list is empty"
  else
    fail "the error list has $count error(s): $(jq -c '.data.rows' <<<"$errors")"
  fi
else
  fail "journals.errors: ${errors:-no answer}"
fi

# memory: still running, not killed, under the limit
if [ "$("$engine" inspect --format '{{.State.Running}}' "$name")" = "true" ]; then
  pass "memory: the container runs under --memory $memory, plugins loaded"
else
  report_dead
  finish
fi
peak_memory

# stop: SIGTERM, which docker stop, podman stop and Kubernetes send, must end the server well before the 10 s after
# which the engine kills it (exit 137)
start=$(date +%s%N)
"$engine" stop "$name" >/dev/null
elapsed_ms=$((($(date +%s%N) - start) / 1000000))
code=$("$engine" inspect --format '{{.State.ExitCode}}' "$name")
if [ "$elapsed_ms" -le $((stop_limit * 1000)) ] && [ "$code" = 0 ]; then
  pass "$engine stop took ${elapsed_ms} ms, exit code $code"
else
  fail "$engine stop took ${elapsed_ms} ms (at most ${stop_limit} s), exit code $code (0 expected): the server does not shut down on SIGTERM"
fi

finish
