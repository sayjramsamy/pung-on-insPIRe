// Baseline benchmark for the insPIRe backend, sweeping the same
// (num_entries, entry_size) combinations as the SealPIR d=2 baseline
// (results/sealpir_baseline_d2.csv) so the two can be compared directly.
//
// For each combination: builds the database, sets up an InspireServer and
// InspireClient once (this pays insPIRe's PackParams/key-generation cost,
// which is legitimately expensive) and records that as setup time, then
// runs 10 queries end-to-end (gen_query -> gen_answer -> decode_answer)
// through the SAME server/client, verifying correctness against the
// plaintext database and recording mean query latency plus query/answer
// sizes.
//
// Writes results/inspire_baseline.csv with columns:
// num_entries, entry_size_bytes, setup_ms, mean_query_ms, query_size_bytes, answer_size_bytes
//
// Usage:
//   inspire_baseline                 # full sweep, writes results/inspire_baseline.csv (unchanged)
//   inspire_baseline <entries> <size> # one combination (size must be 288 or 1024); builds server
//                                      # and client via *_with_breakdown and prints the same
//                                      # per-phase timings as inspire_setup_breakdown, for diagnostic
//                                      # comparison. Does not touch the CSV.

extern crate pung;
extern crate rand;

use pung::pir::inspire::{InspireClient, InspireServer};
use rand::Rng;
use std::env;
use std::fs::File;
use std::io::Write;
use std::time::Instant;

const NUM_QUERIES_PER_COMBO: u64 = 10;

struct Row {
    num_entries: u64,
    entry_size: u64,
    setup_ms: f64,
    mean_query_ms: f64,
    query_size_bytes: usize,
    answer_size_bytes: usize,
}

fn ms(elapsed: std::time::Duration) -> f64 {
    elapsed.as_secs() as f64 * 1000.0 + elapsed.subsec_nanos() as f64 / 1_000_000.0
}

// InspireServer::new<T> is generic over the entry type (mirroring XPIR's
// PirServer::new<T>), so the entry size must be a compile-time constant;
// const generics let the (num_entries, entry_size) sweep still live in one
// loop-free table below instead of duplicated per-size functions.
fn run_combo<const N: usize>(num_entries: u64, rows: &mut Vec<Row>) {
    println!(
        "=== num_entries={} entry_size={} (db={:.2} MiB) ===",
        num_entries,
        N,
        (num_entries * N as u64) as f64 / (1024.0 * 1024.0)
    );

    let mut rng = rand::thread_rng();
    let mut collection: Vec<[u8; N]> = Vec::with_capacity(num_entries as usize);
    for _ in 0..num_entries {
        let mut entry = [0u8; N];
        rng.fill_bytes(&mut entry);
        collection.push(entry);
    }

    let setup_start = Instant::now();
    let server = InspireServer::new(&collection, 1, 1);
    let client = InspireClient::new(N as u64, num_entries, 1, 1);
    let setup_ms = ms(setup_start.elapsed());
    println!("  setup: {:.3} ms", setup_ms);

    let mut query_latencies_ms = Vec::with_capacity(NUM_QUERIES_PER_COMBO as usize);
    let mut query_size_bytes = 0usize;
    let mut answer_size_bytes = 0usize;

    for q in 0..NUM_QUERIES_PER_COMBO {
        let idx = rand::random::<u64>() % num_entries;

        let query_start = Instant::now();
        let query = client.gen_query(idx);
        let answer = server.gen_answer(&query.query, query.num);

        if q == 0 {
            query_size_bytes = query.query.len();
            answer_size_bytes = answer.answer.len();
        }

        let result = client.decode_answer(&answer.answer, answer.num);
        let latency_ms = ms(query_start.elapsed());

        assert_eq!(
            result.result.as_slice(),
            &collection[idx as usize][..],
            "decoded entry mismatch at idx={} (num_entries={}, entry_size={})",
            idx,
            num_entries,
            N
        );

        query_latencies_ms.push(latency_ms);

        println!(
            "  query {}/{}: idx={} latency={:.3} ms",
            q + 1,
            NUM_QUERIES_PER_COMBO,
            idx,
            latency_ms
        );
    }

    let mean_query_ms =
        query_latencies_ms.iter().sum::<f64>() / query_latencies_ms.len() as f64;

    println!(
        "  mean_query={:.3} ms query_size={} bytes answer_size={} bytes",
        mean_query_ms, query_size_bytes, answer_size_bytes
    );

    rows.push(Row {
        num_entries,
        entry_size: N as u64,
        setup_ms,
        mean_query_ms,
        query_size_bytes,
        answer_size_bytes,
    });
}

// Builds server and client via the *_with_breakdown constructors (same code path as
// inspire_setup_breakdown) and prints the same per-phase timings, for a diagnostic, apples-to-apples
// comparison against that binary's output on the same (num_entries, N) combination.
fn run_combo_breakdown<const N: usize>(num_entries: u64) {
    println!(
        "=== num_entries={} entry_size={} (db={:.2} MiB) ===",
        num_entries,
        N,
        (num_entries * N as u64) as f64 / (1024.0 * 1024.0)
    );

    let mut rng = rand::thread_rng();
    let mut collection: Vec<[u8; N]> = Vec::with_capacity(num_entries as usize);
    for _ in 0..num_entries {
        let mut entry = [0u8; N];
        rng.fill_bytes(&mut entry);
        collection.push(entry);
    }

    let t = Instant::now();
    let (server, s) = InspireServer::new_with_breakdown(&collection);
    let server_total = t.elapsed();

    let t = Instant::now();
    let (client, c) = InspireClient::new_with_breakdown(N as u64, num_entries, 1);
    let client_total = t.elapsed();

    let idx = rand::random::<u64>() % num_entries;
    let q = client.gen_query(idx);
    let a = server.gen_answer(&q.query, q.num);
    let r = client.decode_answer(&a.answer, a.num);
    assert_eq!(r.result.as_slice(), &collection[idx as usize][..], "decoded entry mismatch");

    let shape_ms = ms(s.params) + ms(s.pack_params) + ms(c.params) + ms(c.pack_params);
    let data_ms = ms(s.encode) + ms(s.yserver) + ms(s.offline);

    println!("  dim0={} interpolate_degree={}", s.dim0, s.interpolate_degree);
    println!(
        "  server: params={:.1} encode={:.1} yserver={:.1} offline={:.1} pack_params={:.1} total={:.1} ms",
        ms(s.params), ms(s.encode), ms(s.yserver), ms(s.offline), ms(s.pack_params), ms(server_total)
    );
    println!(
        "  client: params={:.1} keygen={:.1} pack_params={:.1} yclient={:.1} total={:.1} ms",
        ms(c.params), ms(c.keygen), ms(c.pack_params), ms(c.yclient), ms(client_total)
    );
    println!("  shape-dependent={:.1} ms  data-dependent={:.1} ms", shape_ms, data_ms);
}

fn run_single(num_entries: u64, size: u64) {
    match size {
        288 => run_combo_breakdown::<288>(num_entries),
        1024 => run_combo_breakdown::<1024>(num_entries),
        _ => panic!("entry size must be 288 or 1024"),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 3 {
        run_single(args[1].parse().unwrap(), args[2].parse().unwrap());
        return;
    }

    let mut rows: Vec<Row> = Vec::new();

    // Same (num_entries, entry_size_bytes) pairs and order as
    // results/sealpir_baseline_d2.csv, for an easy row-by-row diff.
    run_combo::<288>(2048, &mut rows);
    run_combo::<1024>(2048, &mut rows);
    run_combo::<288>(8192, &mut rows);
    run_combo::<1024>(8192, &mut rows);
    run_combo::<288>(32768, &mut rows);
    run_combo::<1024>(32768, &mut rows);
    run_combo::<288>(131072, &mut rows);
    run_combo::<1024>(131072, &mut rows);

    let out_path = "results/inspire_baseline.csv";
    let mut f = File::create(out_path).expect("failed to create results/inspire_baseline.csv");
    writeln!(
        f,
        "num_entries,entry_size_bytes,setup_ms,mean_query_ms,query_size_bytes,answer_size_bytes"
    )
    .unwrap();
    for r in &rows {
        writeln!(
            f,
            "{},{},{:.3},{:.3},{},{}",
            r.num_entries, r.entry_size, r.setup_ms, r.mean_query_ms, r.query_size_bytes, r.answer_size_bytes
        )
        .unwrap();
    }

    println!("Wrote {}", out_path);
}
