// Times each phase of insPIRe setup separately, so the report can say how much of insPIRe's
// per-round setup depends only on the database *shape* (and could be cached across tree levels,
// buckets and rounds) and how much depends on the database *contents* (and must be redone every
// round).
//
// Usage:
//   inspire_setup_breakdown                 # all 8 (num_entries, entry_size) combinations
//   inspire_setup_breakdown <entries> <size> # one combination (size must be 288 or 1024)
//
// Appends one row per combination to results/inspire_setup_breakdown.csv (header written if the
// file is new). Each combination also runs one query and checks the decoded entry, so a broken
// setup cannot produce a timing row.

extern crate pung;
extern crate rand;

use pung::pir::inspire::{InspireClient, InspireServer};
use rand::Rng;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

const OUT: &str = "results/inspire_setup_breakdown.csv";

fn ms(d: Duration) -> f64 {
    d.as_secs() as f64 * 1000.0 + d.subsec_nanos() as f64 / 1_000_000.0
}

fn run_combo<const N: usize>(num_entries: u64) {
    println!("=== num_entries={} entry_size={} ===", num_entries, N);

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

    // Sanity check: one correct retrieval.
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

    let new_file = !Path::new(OUT).exists();
    let mut f = OpenOptions::new().create(true).append(true).open(OUT).expect("open csv");
    if new_file {
        writeln!(
            f,
            "num_entries,entry_size_bytes,dim0,interpolate_degree,\
             s_params_ms,s_encode_ms,s_yserver_ms,s_offline_ms,s_pack_params_ms,s_total_ms,\
             c_params_ms,c_keygen_ms,c_pack_params_ms,c_yclient_ms,c_total_ms,\
             shape_dependent_ms,data_dependent_ms"
        )
        .unwrap();
    }
    writeln!(
        f,
        "{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
        num_entries, N, s.dim0, s.interpolate_degree,
        ms(s.params), ms(s.encode), ms(s.yserver), ms(s.offline), ms(s.pack_params), ms(server_total),
        ms(c.params), ms(c.keygen), ms(c.pack_params), ms(c.yclient), ms(client_total),
        shape_ms, data_ms
    )
    .unwrap();
}

fn run(num_entries: u64, size: u64) {
    match size {
        288 => run_combo::<288>(num_entries),
        1024 => run_combo::<1024>(num_entries),
        _ => panic!("entry size must be 288 or 1024"),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 3 {
        run(args[1].parse().unwrap(), args[2].parse().unwrap());
    } else {
        for &(n, e) in &[
            (2048, 288), (2048, 1024), (8192, 288), (8192, 1024),
            (32768, 288), (32768, 1024), (131072, 288), (131072, 1024),
        ] {
            run(n, e);
        }
    }
    println!("Appended to {}", OUT);
}
