#!/usr/bin/env python3
"""Parse a results/xpir_baseline_*.log into a CSV of latency, sizes, throughput and peak RSS."""
import csv, re, sys

UNIT = {"ns": 1e-6, "us": 1e-3, "µs": 1e-3, "ms": 1.0, "s": 1e3}
NAME = re.compile(r"bench_pir_(query|answer|decode|server_creation)_(\d+)(?:_d_(\d+)_a_(\d+))?(_1KB)?$")
SIZES = {  # alpha for server_creation cases, taken from benches/pir.rs
    ("2048", True): 8, ("8192", True): 8, ("32768", True): 16, ("131072", True): 32,
    ("2048", False): 32, ("8192", False): 32, ("32768", False): 32, ("131072", False): 64,
}

def ms(v, u):
    return float(v) * UNIT[u]

rows, cur = [], None
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    if line.startswith("=== "):
        m = NAME.match(line[4:].strip())
        op, n, d, a, kb = m.groups()
        size = 1024 if kb else 288
        cur = {"case": line[4:].strip(), "op": op, "entries": int(n), "entry_bytes": size,
               "db_mib": round(int(n) * size / 2**20, 2), "d": int(d or 2),
               "alpha": int(a) if a else SIZES[(n, bool(kb))]}
        rows.append(cur)
    elif cur is None:
        continue
    elif (m := re.search(r"time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]", line)):
        cur["lat_ms_low"], cur["lat_ms"], cur["lat_ms_high"] = (
            round(ms(m[1], m[2]), 4), round(ms(m[3], m[4]), 4), round(ms(m[5], m[6]), 4))
    elif (m := re.search(r"(query|answer|decoded result) size: (\d+) bytes", line)):
        cur[m[1].split()[0] + "_bytes"] = int(m[2])
    elif (m := re.search(r"peak_rss_kb=(\d+) wall_s=([\d.]+) exit=(\d+)", line)):
        cur["peak_rss_mib"] = round(int(m[1]) / 1024, 1)
        cur["exit"] = int(m[3])

for r in rows:
    if r["op"] == "answer" and "lat_ms" in r:
        r["server_mib_per_s"] = round(r["db_mib"] / (r["lat_ms"] / 1e3), 1)
        r["answers_per_s"] = round(1e3 / r["lat_ms"], 2)

cols = ["case", "op", "entries", "entry_bytes", "db_mib", "alpha", "d", "lat_ms_low", "lat_ms",
        "lat_ms_high", "query_bytes", "answer_bytes", "decoded_bytes", "server_mib_per_s",
        "answers_per_s", "peak_rss_mib", "exit"]
w = csv.DictWriter(sys.stdout, fieldnames=cols)
w.writeheader()
w.writerows(rows)
