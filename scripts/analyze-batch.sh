#!/usr/bin/env bash
# Analyze many unified degree JSONs — one OS process per file, run JOBS at a
# time, each capped with a virtual-memory limit and a timeout.
#
# Why one process per file: an out-of-memory degree (e.g. a full-catalog scrape
# with thousands of courses) aborts *itself* under the ulimit instead of letting
# the OS OOM-killer take down the whole batch. Threads can't give this — they
# share one address space — so isolation requires separate processes.
#
# Usage:
#   scripts/analyze-batch.sh <input-dir-or-glob> <metrics-dir> [jobs] [-- extra nuanalytics flags]
#
# Examples:
#   scripts/analyze-batch.sh .debug/converted .debug/metrics/json-dump 8
#   scripts/analyze-batch.sh '.debug/converted/*.unified.json' out 4 -- --max-plans 50000
#
# Tunables (env):
#   NA_BIN       analyzer binary           (default: ./target/release/nuanalytics)
#   NA_MEM_KB    per-process vmem cap, KB  (default: 8000000  ~ 8 GB)
#   NA_TIMEOUT   per-process timeout, secs (default: 300)
#
# Keep JOBS * NA_MEM_KB at or below your RAM so concurrent workers don't
# collectively exhaust memory.
set -uo pipefail

INPUT="${1:?usage: analyze-batch.sh <input-dir|glob> <metrics-dir> [jobs] [-- extra flags]}"
NA_METRICS="${2:?metrics-dir required}"
JOBS="${3:-4}"

# Everything after a literal `--` is passed through to `degree analyze`.
shift 3 2>/dev/null || shift "$#"
NA_EXTRA_STR=""
if [ "${1:-}" = "--" ]; then shift; NA_EXTRA_STR="$*"; fi

NA_BIN="${NA_BIN:-./target/release/nuanalytics}"
NA_MEM_KB="${NA_MEM_KB:-8000000}"
NA_TIMEOUT="${NA_TIMEOUT:-300}"

if [ ! -x "$NA_BIN" ]; then
  echo "error: analyzer binary not found at '$NA_BIN'." >&2
  echo "       build it first:  cargo build --release   (or set NA_BIN=...)" >&2
  exit 1
fi

mkdir -p "$NA_METRICS" "$NA_METRICS/logs"
FAILLOG="$NA_METRICS/failures.log"
: > "$FAILLOG"

# Worker: analyze one file in an isolated, memory-capped, time-bounded subshell.
# Success drops its stderr log; failure keeps it and records the path.
analyze_one() {
  local f="$1" base
  base="$(basename "$f")"
  # shellcheck disable=SC2206  # passthrough flags are simple space-free tokens
  local extra=($NA_EXTRA_STR)
  if (
        ulimit -v "$NA_MEM_KB"
        timeout "$NA_TIMEOUT" "$NA_BIN" degree analyze "$f" \
          --no-report --metrics-dir "$NA_METRICS" "${extra[@]}"
      ) >/dev/null 2>"$NA_METRICS/logs/$base.err"; then
    rm -f "$NA_METRICS/logs/$base.err"
  else
    echo "$f" >> "$FAILLOG"
  fi
}
export -f analyze_one
export NA_BIN NA_METRICS NA_MEM_KB NA_TIMEOUT NA_EXTRA_STR FAILLOG

# Collect inputs (a directory, or a glob / space-separated list) and run the pool.
if [ -d "$INPUT" ]; then
  find "$INPUT" -maxdepth 1 -name '*.json' -print0
else
  # shellcheck disable=SC2086  # intentional word-splitting of a glob argument
  printf '%s\0' $INPUT
fi | xargs -0 -r -P "$JOBS" -I{} bash -c 'analyze_one "$@"' _ {}

produced=$(find "$NA_METRICS" -maxdepth 1 -name '*_report.json' | wc -l | tr -d ' ')
failed=$(wc -l < "$FAILLOG" | tr -d ' ')
echo "----------------------------------------"
echo "reports produced : $produced"
echo "failed/skipped   : $failed   (see $FAILLOG; per-file stderr in $NA_METRICS/logs/)"
