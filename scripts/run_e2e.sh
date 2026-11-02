#!/usr/bin/env bash
# End-to-end Pung measurement: SealPIR vs insPIRe, real server+client rounds over Pung's RPC.
# See ~/e2e_spec.md for the full spec. Runs three parts in order, never in parallel:
#   A. main sweep: explicit retrieval, both backends, EXTRA in {2048, 8192, 32768, 131072}
#   B. tree retrieval demonstration: both backends, EXTRA=2048 only (small, but insPIRe pays
#      per-level setup, so still ~20-30 min)
#   C. SealPIR peak memory via the sealpir_baseline microbenchmark under /usr/bin/time -v
#
# Usage: nohup bash scripts/run_e2e.sh > results/e2e/progress.txt 2>&1 &
# (tmux does not work on this machine, hence nohup.)
set -euo pipefail

cd "$(dirname "$0")/.."

OUT=results/e2e
mkdir -p "$OUT"

SEALPIR_SERVER=target/release/server
SEALPIR_CLIENT=target/release/client
INSPIRE_SERVER=target-inspire/release/server
INSPIRE_CLIENT=target-inspire/release/client
PORT=12345

SERVER_PID=""

ts() { date -u +"%Y-%m-%dT%H:%M:%SZ"; }
progress() { echo "[$(ts)] $*"; }
fail() {
    echo "[$(ts)] FAIL: $*" >&2
    exit 1
}

cleanup() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Refuse to start if another build or benchmark is already running on this machine (it would
# distort timings).
if pgrep -af "run_session|inspire_|sealpir_|target/release" >/dev/null; then
    fail "another benchmark/build process is already running; aborting. Check with: pgrep -af \"run_session|inspire_|sealpir_|target/release\""
fi

wait_for_server() {
    local port="$1" tries=0
    # Each attempt opens/closes its own fd 3 inside a subshell, so nothing leaks into this shell.
    until (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; do
        tries=$((tries + 1))
        if [ "$tries" -ge 60 ]; then
            fail "server did not start listening on port $port within 60s"
        fi
        sleep 1
    done
}

start_server() {
    local server_bin="$1" scheme="$2" log="$3"
    "$server_bin" -m 1 -t "$scheme" > "$log" 2>&1 &
    SERVER_PID=$!
    wait_for_server "$PORT"
}

stop_server() {
    if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    SERVER_PID=""
}

check_rounds() {
    local log="$1" expected="$2" label="$3" got
    got=$(grep -c "Retrieved msg" "$log" || true)
    if [ "$got" -ne "$expected" ]; then
        fail "$label: expected $expected 'Retrieved msg' lines in $log, got $got"
    fi
}

backend_bins() {
    case "$1" in
        sealpir) echo "$SEALPIR_SERVER" "$SEALPIR_CLIENT" ;;
        inspire) echo "$INSPIRE_SERVER" "$INSPIRE_CLIENT" ;;
        *) fail "unknown backend $1" ;;
    esac
}

run_combo() {
    local backend="$1" scheme="$2" extra="$3" rounds="$4" expected="$5"
    local server_bin client_bin
    read -r server_bin client_bin <<< "$(backend_bins "$backend")"

    local server_log="$OUT/${backend}_${scheme}_${extra}_server.log"
    local client_log="$OUT/${backend}_${scheme}_${extra}_client.log"

    progress "backend=$backend scheme=$scheme extra=$extra rounds=$rounds"

    start_server "$server_bin" "$scheme" "$server_log"

    if ! "$client_bin" -n u1 -p u1 -x secret -r "$rounds" -t "$scheme" -b "$extra" \
        > "$client_log" 2>&1; then
        stop_server
        fail "client exited non-zero for backend=$backend scheme=$scheme extra=$extra (see $client_log)"
    fi

    stop_server
    check_rounds "$client_log" "$expected" "backend=$backend scheme=$scheme extra=$extra"
}

progress "Part A: main sweep (explicit retrieval)"
for backend in sealpir inspire; do
    for extra in 2048 8192 32768 131072; do
        run_combo "$backend" e "$extra" 4 4
    done
done

progress "Part B: tree retrieval demonstration (small)"
for backend in sealpir inspire; do
    run_combo "$backend" t 2048 2 2
done

progress "Part C: SealPIR peak memory"
/usr/bin/time -v -o "$OUT/time_sealpir_baseline.txt" ./target/release/sealpir_baseline \
    > "$OUT/sealpir_baseline_memory_run.log" 2>&1
mv results/sealpir_baseline_d2.csv "$OUT/sealpir_baseline_memory_run.csv"
git checkout -- results/sealpir_baseline_d2.csv

progress "Recording machine info"
{
    echo "### lscpu model name"
    lscpu | grep "Model name"
    echo "### free -h"
    free -h
    echo "### git commit"
    git rev-parse HEAD
} > "$OUT/machine_info.txt"

progress "All parts complete."
