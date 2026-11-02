// Records the size of the Galois keys a SealPIR client must upload to the server once (they are
// not part of the per-query sizes in results/sealpir_baseline_r5.csv). Same parameters and the
// same 8 (num_entries, entry_size) combinations as the SealPIR baseline.
//
// Writes results/sealpir_galois_keys.csv.

extern crate pung;

use pung::pir::sealpir_client::SealPirClient;
use std::fs::File;
use std::io::Write;

fn main() {
    let combos: &[(u64, u64)] = &[
        (2048, 288), (2048, 1024), (8192, 288), (8192, 1024),
        (32768, 288), (32768, 1024), (131072, 288), (131072, 1024),
    ];
    let out = "results/sealpir_galois_keys.csv";
    let mut f = File::create(out).expect("create csv");
    writeln!(f, "num_entries,entry_size_bytes,galois_keys_bytes").unwrap();
    for &(n, e) in combos {
        let client = SealPirClient::new(n, e);
        let keys = client.get_galois_keys();
        println!("num_entries={} entry_size={} galois_keys={} bytes ({:.1} KiB)", n, e, keys.len(), keys.len() as f64 / 1024.0);
        writeln!(f, "{},{},{}", n, e, keys.len()).unwrap();
    }
    println!("Wrote {}", out);
}
