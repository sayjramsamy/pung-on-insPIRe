// Baseline benchmark for the SealPIR backend, sweeping the same
// (num_entries, entry_size) combinations as the XPIR baseline
// (results/xpir_baseline_avx512.csv) so the two can be compared directly.
//
// For each combination: builds the database, sets up a SealPirServer and
// SealPirClient once (including the galois-key handshake) and records that
// as setup time, then runs 10 queries end-to-end (gen_query -> gen_answer ->
// decode_answer), verifying correctness against the plaintext database and
// recording mean query latency plus query/answer sizes.
//
// Writes results/sealpir_baseline_d2.csv with columns:
// num_entries, entry_size_bytes, setup_ms, mean_query_ms, query_size_bytes, answer_size_bytes

extern crate pung;
extern crate rand;

use pung::pir::sealpir_client::SealPirClient;
use pung::pir::sealpir_server::SealPirServer;
use std::fs::File;
use std::io::Write;
use std::time::Instant;

const NUM_QUERIES_PER_COMBO: u64 = 10;

// Same (num_entries, entry_size_bytes) pairs as results/xpir_baseline_avx512.csv.
const COMBOS: &[(u64, u64)] = &[
    (2048, 288),
    (2048, 1024),
    (8192, 288),
    (8192, 1024),
    (32768, 288),
    (32768, 1024),
    (131072, 288),
    (131072, 1024),
];

fn ms(elapsed: std::time::Duration) -> f64 {
    elapsed.as_secs() as f64 * 1000.0 + elapsed.subsec_nanos() as f64 / 1_000_000.0
}

fn main() {
    let mut rows: Vec<(u64, u64, f64, f64, usize, usize)> = Vec::with_capacity(COMBOS.len());

    for &(num_entries, entry_size) in COMBOS {
        println!(
            "=== num_entries={} entry_size={} (db={:.2} MiB) ===",
            num_entries,
            entry_size,
            (num_entries * entry_size) as f64 / (1024.0 * 1024.0)
        );

        let total_bytes = (num_entries * entry_size) as usize;
        let mut db = vec![0u8; total_bytes];
        for (i, b) in db.iter_mut().enumerate() {
            *b = (i % 256) as u8;
        }

        let setup_start = Instant::now();
        let server = SealPirServer::new(&db, num_entries, entry_size);
        let client = SealPirClient::new(num_entries, entry_size);
        let galois_keys = client.get_galois_keys();
        server.set_galois_keys(&galois_keys);
        let setup_ms = ms(setup_start.elapsed());
        println!("  setup: {:.3} ms", setup_ms);

        let mut query_latencies_ms = Vec::with_capacity(NUM_QUERIES_PER_COMBO as usize);
        let mut query_size_bytes = 0usize;
        let mut answer_size_bytes = 0usize;

        for q in 0..NUM_QUERIES_PER_COMBO {
            let idx = rand::random::<u64>() % num_entries;

            let query_start = Instant::now();
            let query = client.gen_query(idx);
            let answer = server.gen_answer(&query);

            if q == 0 {
                query_size_bytes = query.to_bytes().len();
                answer_size_bytes = answer.to_bytes().len();
            }

            let result = client.decode_answer(answer);
            let latency_ms = ms(query_start.elapsed());

            let decoded = result.to_bytes();
            let expected_start = (idx * entry_size) as usize;
            let expected = &db[expected_start..expected_start + entry_size as usize];
            assert_eq!(
                decoded.as_slice(),
                expected,
                "decoded entry mismatch at idx={} (num_entries={}, entry_size={})",
                idx,
                num_entries,
                entry_size
            );

            query_latencies_ms.push(latency_ms);

            println!("  query {}/{}: idx={} latency={:.3} ms", q + 1, NUM_QUERIES_PER_COMBO, idx, latency_ms);
        }

        let mean_query_ms =
            query_latencies_ms.iter().sum::<f64>() / query_latencies_ms.len() as f64;

        println!(
            "  mean_query={:.3} ms query_size={} bytes answer_size={} bytes",
            mean_query_ms, query_size_bytes, answer_size_bytes
        );

        rows.push((
            num_entries,
            entry_size,
            setup_ms,
            mean_query_ms,
            query_size_bytes,
            answer_size_bytes,
        ));
    }

    let out_path = "results/sealpir_baseline_d2.csv";
    let mut f = File::create(out_path).expect("failed to create results/sealpir_baseline_d2.csv");
    writeln!(
        f,
        "num_entries,entry_size_bytes,setup_ms,mean_query_ms,query_size_bytes,answer_size_bytes"
    )
    .unwrap();
    for (num_entries, entry_size, setup_ms, mean_query_ms, query_size, answer_size) in rows {
        writeln!(
            f,
            "{},{},{:.3},{:.3},{},{}",
            num_entries, entry_size, setup_ms, mean_query_ms, query_size, answer_size
        )
        .unwrap();
    }

    println!("Wrote {}", out_path);
}
