#!/usr/bin/env bash
# Runs the SealPIR baseline benchmark (src/bin/sealpir_baseline.rs), sweeping
# the same (num_entries, entry_size) combinations as the XPIR baseline
# (results/xpir_baseline_avx512.csv). Writes results/sealpir_baseline.csv.
#
# Usage: scripts/run_sealpir_baseline.sh
set -eu

cd "$(dirname "$0")/.."

echo "# SealPIR baseline, $(date -Is)"
echo "# host: $(nproc) cores, $(free -h | awk '/Mem/{print $2}') RAM, $(uname -r)"
echo "# rustc: $(rustc --version); commit: $(git rev-parse --short HEAD)"

cargo build --release --bin sealpir_baseline

./target/release/sealpir_baseline

echo "# done $(date -Is)"
