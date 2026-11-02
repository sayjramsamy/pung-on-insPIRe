#!/usr/bin/env bash
# Extra measurements for the report. Run from the repository root on the r5.xlarge:
#   bash scripts/run_extra_measurements.sh
# Takes roughly 20-25 minutes after the build. Safe to run inside `tmux` so an SSH drop
# doesn't kill it.
set -euo pipefail
mkdir -p results
command -v /usr/bin/time >/dev/null || sudo apt-get install -y time

echo "== 1/4 machine info"
{
  echo "### date"; date -u
  echo "### lscpu"; lscpu
  echo "### os-release"; cat /etc/os-release
  echo "### memory"; free -h
  echo "### avx512 flags"; grep -o 'avx512[a-z_]*' /proc/cpuinfo | sort -u | tr '\n' ' '; echo
  echo "### toolchain"; rustc --version; cargo --version
  echo "### pung commit"; git rev-parse HEAD
  echo "### inspire commit"; git -C ../private-membership log -1 --format='%H %d %s' || true
} > results/machine_info.txt
echo "   wrote results/machine_info.txt"

echo "== 2/4 build"
cargo build --release --bin sealpir_keysize --bin inspire_setup_breakdown

echo "== 3/4 SealPIR Galois key sizes (a few seconds)"
./target/release/sealpir_keysize | tee results/sealpir_galois_keys.log

echo "== 4/4 insPIRe setup breakdown, one process per combination (~20 min)"
rm -f results/inspire_setup_breakdown.csv results/inspire_setup_breakdown.log
for combo in "2048 288" "2048 1024" "8192 288" "8192 1024" \
             "32768 288" "32768 1024" "131072 288" "131072 1024"; do
  set -- $combo
  /usr/bin/time -v -o "results/time_${1}_${2}.txt" \
    ./target/release/inspire_setup_breakdown "$1" "$2" | tee -a results/inspire_setup_breakdown.log
  grep -E "Maximum resident set size|Percent of CPU" "results/time_${1}_${2}.txt" \
    | sed "s/^/   [$1 x $2] /" | tee -a results/inspire_setup_breakdown.log
done

echo
echo "Done. New files in results/:"
ls -1 results/machine_info.txt results/sealpir_galois_keys.* results/inspire_setup_breakdown.* results/time_*.txt
