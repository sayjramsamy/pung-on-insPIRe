#!/usr/bin/env python3
"""Parses results/e2e/*_server.log and *_client.log (written by scripts/run_e2e.sh) into
results/e2e/summary.csv and results/e2e/tree_levels.csv.

Usage: python3 scripts/parse_e2e.py
"""
import csv
import re
from pathlib import Path

OUT = Path("results/e2e")

RE_PIR_SETUP_LEVEL = re.compile(r"^pir_setup_level level=(\d+) tuples=(\d+) usec=(\d+)$")
RE_PIR_SETUP_ROUND = re.compile(r"^pir_setup round=(\d+) tuples=(\d+) usec=(\d+)$")
RE_KEYS_TIME = re.compile(r"^keys_time level=(\d+) usec=(\d+)$")
RE_ANSWER_TIME = re.compile(r"^answer_time level=(\d+) usec=(\d+)$")
RE_ADVANCING = re.compile(r"^Advancing to round (\d+)$")

RE_UPLOAD_SEND = re.compile(r"^Upload \(send rpc\) (\d+) bytes$")
RE_SEND = re.compile(r"^send \(\d+ msgs\): (\d+) usec$")
RE_RETR = re.compile(r"^retr \(\d+ msgs\): (\d+) usec$")
RE_UPLOAD_PIR = re.compile(r"^Upload \(pir\) (\d+) bytes$")
RE_UPLOAD_KEYS = re.compile(r"^Upload \(keys\) (\d+) bytes$")
RE_DOWNLOAD_PIR = re.compile(r"^Download \(pir\) (\d+) bytes$")


def parse_server_log(path):
    """One dict per round, in the order the server printed 'pir_setup round=':
    {pir_setup_usec, tuples, levels: [(level, tuples, pir_setup_usec), ...],
     answer_by_level: {level: usec}, keys_by_level: {level: usec}}.

    pir_setup_level lines for round R are printed (by db::Collection::pir_setup) before the
    'pir_setup round=R' line that follows them; answer_time/keys_time lines for round R are
    printed (by server::rpc::retr) after it, up to the 'Advancing to round R+1' line.
    """
    rounds = []
    level_setup_buf = []
    cur = None

    for raw in path.read_text().splitlines():
        line = raw.strip()

        m = RE_PIR_SETUP_LEVEL.match(line)
        if m:
            level_setup_buf.append((int(m.group(1)), int(m.group(2)), int(m.group(3))))
            continue

        m = RE_PIR_SETUP_ROUND.match(line)
        if m:
            cur = {
                "pir_setup_usec": int(m.group(3)),
                "tuples": int(m.group(2)),
                "levels": level_setup_buf,
                "answer_by_level": {},
                "keys_by_level": {},
            }
            level_setup_buf = []
            rounds.append(cur)
            continue

        m = RE_KEYS_TIME.match(line)
        if m and cur is not None:
            cur["keys_by_level"][int(m.group(1))] = int(m.group(2))
            continue

        m = RE_ANSWER_TIME.match(line)
        if m and cur is not None:
            cur["answer_by_level"][int(m.group(1))] = int(m.group(2))
            continue

        if RE_ADVANCING.match(line):
            cur = None
            continue

    return rounds


def parse_client_log(path):
    """One dict per round, in the order the client printed 'Upload (send rpc)':
    {send_usec, retr_usec, upload_pir_bytes, upload_keys_bytes, download_pir_bytes}.
    Upload (pir) / Upload (keys) / Download (pir) are summed across levels within the round.
    """
    rounds = []
    cur = None

    for raw in path.read_text().splitlines():
        line = raw.strip()

        if RE_UPLOAD_SEND.match(line):
            cur = {
                "send_usec": 0,
                "retr_usec": 0,
                "upload_pir_bytes": 0,
                "upload_keys_bytes": 0,
                "download_pir_bytes": 0,
            }
            rounds.append(cur)
            continue

        if cur is None:
            continue

        m = RE_SEND.match(line)
        if m:
            cur["send_usec"] = int(m.group(1))
            continue

        m = RE_RETR.match(line)
        if m:
            cur["retr_usec"] = int(m.group(1))
            continue

        m = RE_UPLOAD_PIR.match(line)
        if m:
            cur["upload_pir_bytes"] += int(m.group(1))
            continue

        m = RE_UPLOAD_KEYS.match(line)
        if m:
            cur["upload_keys_bytes"] += int(m.group(1))
            continue

        m = RE_DOWNLOAD_PIR.match(line)
        if m:
            cur["download_pir_bytes"] += int(m.group(1))
            continue

    return rounds


def process_combo(backend, scheme, size, server_log, client_log, summary_rows, tree_rows):
    if not server_log.exists() or not client_log.exists():
        print(f"skip {backend} {scheme} {size}: missing log(s) ({server_log}, {client_log})")
        return

    server_rounds = parse_server_log(server_log)
    client_rounds = parse_client_log(client_log)

    if len(server_rounds) != len(client_rounds):
        print(
            f"skip {backend} {scheme} {size}: server/client round count mismatch "
            f"({len(server_rounds)} vs {len(client_rounds)})"
        )
        return

    for i, (sr, cr) in enumerate(zip(server_rounds, client_rounds)):
        round_num = i + 1  # 1-indexed; round 1 is the warm-up round (see e2e_spec.md)
        answer_usec = sum(sr["answer_by_level"].values())
        keys_usec = sum(sr["keys_by_level"].values())

        summary_rows.append([
            backend, scheme, size, round_num,
            sr["pir_setup_usec"], answer_usec, keys_usec,
            cr["retr_usec"], cr["send_usec"],
            cr["upload_pir_bytes"], cr["upload_keys_bytes"], cr["download_pir_bytes"],
            len(sr["levels"]),
        ])

        if scheme == "t":
            for level, tuples, pir_setup_usec in sr["levels"]:
                level_answer_usec = sr["answer_by_level"].get(level, 0)
                level_keys_usec = sr["keys_by_level"].get(level, 0)
                tree_rows.append(
                    [backend, round_num, level, tuples, pir_setup_usec, level_answer_usec, level_keys_usec]
                )


def write_csv(path, header, rows):
    with path.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(header)
        w.writerows(rows)
    print(f"wrote {path} ({len(rows)} rows)")


def main():
    summary_rows = []
    tree_rows = []

    for backend in ("sealpir", "inspire"):
        for size in (2048, 8192, 32768, 131072):
            process_combo(
                backend, "e", size,
                OUT / f"{backend}_e_{size}_server.log",
                OUT / f"{backend}_e_{size}_client.log",
                summary_rows, tree_rows,
            )
        process_combo(
            backend, "t", 2048,
            OUT / f"{backend}_t_2048_server.log",
            OUT / f"{backend}_t_2048_client.log",
            summary_rows, tree_rows,
        )

    write_csv(
        OUT / "summary.csv",
        [
            "backend", "scheme", "size", "round",
            "pir_setup_usec", "answer_usec", "keys_usec",
            "retr_usec", "send_usec",
            "upload_pir_bytes", "upload_keys_bytes", "download_pir_bytes",
            "levels",
        ],
        summary_rows,
    )
    write_csv(
        OUT / "tree_levels.csv",
        ["backend", "round", "level", "tuples", "pir_setup_usec", "answer_usec", "keys_usec"],
        tree_rows,
    )


if __name__ == "__main__":
    main()
