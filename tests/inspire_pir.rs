extern crate rand;
extern crate pung;

use pung::pir::inspire::{debug_packed_row_query, InspireClient, InspireServer};
use rand::Rng;

#[test]
fn inspire_pir_roundtrip() {
    let num: u64 = 16;
    let size: usize = 288;

    let mut rng = rand::thread_rng();
    let mut collection: Vec<[u8; 288]> = Vec::new();
    for _ in 0..num {
        let mut x = [0u8; 288];
        rng.fill_bytes(&mut x);
        collection.push(x);
    }

    let server = InspireServer::new(&collection, 1, 1);
    let client = InspireClient::new(size as u64, num, 1, 1);

    for idx in [0u64, 1, 7, 15] {
        let query = client.gen_query(idx);
        let answer = server.gen_answer(&query.query, query.num);
        let result = client.decode_answer(&answer.answer, answer.num);
        assert_eq!(result.result.as_slice(), &collection[idx as usize][..]);
    }
}

/// insPIRe's query encrypts the requested row under public randomness from a *fixed* seed
/// (`SEED_0`), so the CRS is identical across queries to the same database shape. If the client
/// reused one secret key across queries, two queries for the same index would then differ only by
/// fresh encryption noise (small, once centred mod the ciphertext modulus). With a fresh secret key
/// per query (the fix in `InspireClient::gen_query`), the same two queries differ by
/// `(sk_1 - sk_2) * a + noise`, where `a` is the fixed public mask: since `a` is uniformly random
/// and `sk_1 != sk_2` with overwhelming probability, that term is not small.
#[test]
fn inspire_pir_fresh_key_per_query() {
    let num: u64 = 16;
    let size: u64 = 288;
    let idx: u64 = 3;

    let client = InspireClient::new(size, num, 1, 1);

    let query_a = client.gen_query(idx);
    let query_b = client.gen_query(idx);

    let (row_a, modulus) = debug_packed_row_query(&query_a, num, size);
    let (row_b, modulus_b) = debug_packed_row_query(&query_b, num, size);
    assert_eq!(modulus, modulus_b);
    assert_eq!(row_a.len(), row_b.len());

    // Noise terms in this scheme are tiny relative to the modulus (bounded by a handful of
    // standard deviations of a discrete Gaussian with small width); a `(sk_1 - sk_2) * a` term is
    // essentially uniform mod `modulus`. So a centred difference clearing this threshold anywhere
    // in the row can only come from a changed key, never from noise alone.
    let noise_threshold = modulus / 1_000_000;

    let max_centered_abs_diff = row_a
        .iter()
        .zip(row_b.iter())
        .map(|(&a, &b)| {
            let crt_a = a % modulus;
            let crt_b = b % modulus;
            let diff = (crt_a + modulus - crt_b) % modulus;
            diff.min(modulus - diff)
        })
        .max()
        .unwrap();

    assert!(
        max_centered_abs_diff > noise_threshold,
        "row-query difference for repeated index {} looked like pure noise \
         (max centred |diff| = {}, modulus = {}); secret key may be getting reused across queries",
        idx,
        max_centered_abs_diff,
        modulus
    );
}
