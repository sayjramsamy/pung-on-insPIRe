#!/usr/bin/env bash
# Runs each insPIRe PIR benchmark in benches/inspire_pir.rs as its own process (sequentially),
# recording criterion output and peak RSS. Mirrors run_xpir_baseline.sh so the two logs parse
# identically. Usage: results/run_inspire_baseline.sh <binary>
set -u
BIN="$1"
OUT="$(dirname "$0")/inspire_benchmark_$(date +%Y%m%d_%H%M).log"
{
  echo "# insPIRe baseline, $(date -Is)"
  echo "# host: $(nproc) cores, $(free -h | awk '/Mem/{print $2}') RAM, $(uname -r)"
  echo "# rustc: $(rustc --version); commit: $(git -C "$(dirname "$0")/.." rev-parse --short HEAD)"
  echo "# note: insPIRe ignores alpha/d (see XPIR_API.md); case names/sizes mirror benches/pir.rs for comparison only"
} > "$OUT"
for t in $("$BIN" --list 2>/dev/null | sed -n 's/: test$//p'); do
  echo "=== $t" >> "$OUT"
  /usr/bin/time -f "peak_rss_kb=%M wall_s=%e exit=%x" "$BIN" --exact --test-threads=1 --nocapture "$t" >> "$OUT" 2>&1
done
echo "# done $(date -Is)" >> "$OUT"
echo "$OUT"
