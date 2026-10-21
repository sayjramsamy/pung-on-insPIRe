extern crate rand;
extern crate pung;

use pung::pir::inspire::{InspireClient, InspireServer};
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
