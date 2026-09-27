// insPIRe backend for Pung's PIR interface (skeleton; no logic yet).
//
// Mirrors the API of `pir_server::PirServer` and `pir_client::PirClient` so that callers can switch
// backends without changes. The nine operations of the XPIR C ABI (see XPIR_API.md) map as follows:
//
// | XPIR C ABI                    | here                                                     |
// |-------------------------------|----------------------------------------------------------|
// | cpp_server_setup              | InspireServer::new                                       |
// | cpp_server_process_query      | InspireServer::gen_answer                                |
// | cpp_server_free               | Drop for InspireServer                                   |
// | cpp_client_setup              | InspireClient::new                                       |
// | cpp_client_update_db_params   | InspireClient::update_params                             |
// | cpp_client_generate_query     | InspireClient::gen_query                                 |
// | cpp_client_process_reply      | InspireClient::decode_answer                             |
// | cpp_client_free               | Drop for ClientDbState (per-database client state)       |
// | cpp_buffer_free               | none: InspireQuery/InspireAnswer/InspireResult own Vecs  |
//
// Ownership: insPIRe's YServer, YClient and OfflinePrecomputedValues borrow `&'a Params` (and
// YClient borrows `&'a mut Client<'a>`), so a struct cannot simply own both the Params and the
// things that borrow it. The plan is to leak the Params (and Client) with `Box::leak` so the
// borrowers can be `'static`, and reclaim them in `Drop` after the borrowers are dropped; the
// borrowers are wrapped in `ManuallyDrop` so `Drop` controls that order.

// The skeleton declares state and signatures that nothing uses yet.
#![allow(dead_code, unused_imports, unused_variables)]

use std::cell::RefCell;
use std::mem::ManuallyDrop;

use inspire::client::YClient;
use inspire::pung::*;
use inspire::server::{OfflinePrecomputedValues, YServer};
use spiral_rs::params::Params;

/// Server side: one insPIRe database for one level of a collection, rebuilt every round.
pub struct InspireServer {
    /// Leaked in `new` so `server` and `offline` can borrow it for `'static`; reclaimed in `Drop`.
    params: &'static Params,
    /// Encoded (interpolated) database.
    server: ManuallyDrop<YServer<'static, u16>>,
    /// Result of the offline (silent) preprocessing, reused by every query this round.
    offline: ManuallyDrop<OfflinePrecomputedValues<'static>>,
    /// Records per row that the RGSW ciphertext selects between.
    interpolate_degree: usize,
    /// Number of Pung tuples in this database.
    num_entries: u64,
    /// Size of each Pung tuple in bytes.
    entry_bytes: u64,
}

impl InspireServer {
    /// Encodes `collection` and runs the offline precomputation (cpp_server_setup).
    pub fn new<T>(collection: &[T], alpha: u64, depth: u64) -> InspireServer {
        todo!()
    }

    /// Answers one serialized query (cpp_server_process_query).
    pub fn gen_answer(&self, query: &[u8], q_num: u64) -> InspireAnswer {
        todo!()
    }
}

impl Drop for InspireServer {
    /// Drops `offline` and `server`, then reclaims the leaked `params` (cpp_server_free).
    fn drop(&mut self) {
        todo!()
    }
}

/// Client side. One client is reused across every database it queries; `update_params` switches it
/// to the next database's shape before each query.
pub struct InspireClient {
    depth: u64,
    /// State for the current database. `RefCell` because Pung calls every method through `&self`.
    db: RefCell<Option<ClientDbState>>,
}

/// Client state for one database shape: parameters, secret key, and the index being retrieved.
struct ClientDbState {
    /// Leaked so `y_client` can borrow it for `'static`; reclaimed in `Drop`.
    params: &'static Params,
    /// Wraps the leaked spiral-rs `Client` holding the secret key.
    y_client: ManuallyDrop<YClient<'static>>,
    interpolate_degree: usize,
    num_entries: u64,
    entry_bytes: u64,
    /// Index passed to the last `gen_query`, needed by `decode_answer` to extract the record.
    last_index: Option<u64>,
}

impl Drop for ClientDbState {
    /// Drops `y_client`, then reclaims the leaked `Client` and `params` (cpp_client_free).
    fn drop(&mut self) {
        todo!()
    }
}

impl InspireClient {
    /// Creates a client for a database of `num` entries of `size` bytes (cpp_client_setup).
    pub fn new(size: u64, num: u64, alpha: u64, depth: u64) -> InspireClient {
        todo!()
    }

    /// Switches the client to a database of `num` entries of `size` bytes
    /// (cpp_client_update_db_params).
    pub fn update_params(&self, size: u64, num: u64, alpha: u64) {
        todo!()
    }

    /// Builds the serialized query for entry `index` (cpp_client_generate_query).
    pub fn gen_query(&self, index: u64) -> InspireQuery {
        todo!()
    }

    /// Decrypts `answer` and extracts the entry requested by the last `gen_query`
    /// (cpp_client_process_reply).
    pub fn decode_answer(&self, answer: &[u8], a_num: u64) -> InspireResult {
        todo!()
    }
}

/// Serialized query: packing keys, row query and RGSW ciphertext, as `num` equal-size elements.
pub struct InspireQuery {
    pub query: Vec<u8>,
    pub num: u64,
}

/// Serialized answer: the mod-switched response ciphertexts, as `num` equal-size elements.
pub struct InspireAnswer {
    pub answer: Vec<u8>,
    pub num: u64,
}

/// The decoded entry, exactly `size` bytes.
pub struct InspireResult {
    pub result: Vec<u8>,
}
