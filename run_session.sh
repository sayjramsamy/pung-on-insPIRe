#!/usr/bin/env bash
set -euo pipefail
cd ~/pung-on-insPIRe
OUT=results/session_r5
mkdir -p "$OUT"
cargo build --release --bin sealpir_baseline --bin inspire_baseline --bin sealpir_keysize --bin inspire_setup_breakdown
{ date -u; lscpu; cat /etc/os-release; free -h; git rev-parse HEAD; git -C ../private-membership log -1 --format='%H %s' || true; } > "$OUT/machine_info.txt"

echo "1/4 SealPIR baseline  $(date -u +%T)"
./target/release/sealpir_baseline > "$OUT/sealpir_baseline.log" 2>&1
mv results/sealpir_baseline_d2.csv "$OUT/sealpir_baseline.csv"; git checkout -- results/sealpir_baseline_d2.csv

echo "2/4 SealPIR key sizes $(date -u +%T)"
./target/release/sealpir_keysize > "$OUT/sealpir_galois_keys.log" 2>&1
mv results/sealpir_galois_keys.csv "$OUT/"

echo "3/4 insPIRe baseline  $(date -u +%T)"
./target/release/inspire_baseline > "$OUT/inspire_baseline.log" 2>&1
mv results/inspire_baseline.csv "$OUT/inspire_baseline.csv"; git checkout -- results/inspire_baseline.csv

echo "4/4 insPIRe breakdown $(date -u +%T)"
rm -f results/inspire_setup_breakdown.csv
for combo in "2048 288" "2048 1024" "8192 288" "8192 1024" "32768 288" "32768 1024" "131072 288" "131072 1024"; do
  set -- $combo
  echo "    $1 x $2  $(date -u +%T)"
  /usr/bin/time -v -o "$OUT/time_$1_$2.txt" ./target/release/inspire_setup_breakdown "$1" "$2" >> "$OUT/inspire_setup_breakdown.log" 2>&1
done
mv results/inspire_setup_breakdown.csv "$OUT/"
echo "DONE $(date -u +%T)"
