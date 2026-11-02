// insPIRe backend for Pung's PIR interface.
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
//
// Record encoding: insPIRe's plaintext elements are reduced mod 65535 (not 65536), so a raw 16-bit
// word cannot be packed directly -- 0xFFFF would decode as 0. Rather than pack 15 bits per element,
// this wrapper packs one byte per element (always < 256), trading up to 2x rate for simplicity.

use std::cell::RefCell;
use std::mem::{self, ManuallyDrop};
use std::slice;
use std::time::{Duration, Instant};

use inspire::client::{pack_query, YClient};
use inspire::commons::{deserialize_everything, params_rgswpir_given_input_size_and_dim0, serialize_everything};
use inspire::packing::{PackParams, PackingType};
use inspire::pung::{decode_rgsw_response, generate_packing_keys, generate_rgsw_query, index_to_row_sub_col, RGSWPIR};
use inspire::scheme::SEED_0;
use inspire::server::{OfflinePrecomputedValues, YServer};
use spiral_rs::client::Client;
use spiral_rs::params::Params;

/// insPIRe hard-codes `poly_len_log2 = 11` in `params_rgswpir_given_input_size_and_dim0`.
const POLY_LEN: u64 = 2048;
/// `db_rows.next_power_of_two().trailing_zeros() - poly_len_log2` underflows if `db_rows < POLY_LEN`,
/// so `dim0` must be at least this large regardless of database size.
const MIN_DIM0: u64 = POLY_LEN;

/// Picks the first-dimension size for `params_rgswpir_given_input_size_and_dim0`, balancing the
/// first-dimension query (grows with `dim0`) against the RGSW/interpolation cost (grows with
/// `logical_items / dim0`) by targeting `dim0 ~= sqrt(logical_items)`, floored at `MIN_DIM0`.
fn choose_dim0(num_entries: u64, entry_bytes: u64) -> usize {
    let factor = (POLY_LEN / entry_bytes.max(1)).max(1);
    let logical_items = num_entries.max(1).div_ceil(factor);
    let balanced = (logical_items as f64).sqrt().ceil() as u64;
    balanced.max(MIN_DIM0).next_power_of_two() as usize
}

/// Where a raw entry lives in the database that `encode_db` builds: `row` is selected by the
/// first-dimension query, `sub_col` by the RGSW ciphertext; `group_idx` and `within_offset` locate
/// the entry's bytes inside the `gamma` decoded elements for that `sub_col`.
struct Location {
    row: usize,
    sub_col: usize,
    group_idx: usize,
    within_offset: usize,
}

/// Locates raw entry `idx` within a database built for `num_entries` of `entry_bytes` bytes each
/// (identical on both client and server, since both derive `params` from the same two numbers).
fn locate(params: &Params, interpolate_degree: usize, entry_bytes: u64, idx: u64) -> Location {
    let gamma = params.poly_len;
    let db_cols = params.instances * gamma;
    let per = params.instances as u64;
    let factor = (gamma as u64 / entry_bytes.max(1)).max(1);

    let logical_idx = idx / factor;
    let within_offset = ((idx % factor) * entry_bytes) as usize;
    let col_logical = (logical_idx % per) as usize;
    let row = (logical_idx / per) as usize;
    let col_start = col_logical * gamma + within_offset;

    let (row_from_lib, sub_col) =
        index_to_row_sub_col(row * db_cols + col_start, db_cols, interpolate_degree, gamma);
    debug_assert_eq!(row_from_lib, row);

    Location {
        row,
        sub_col,
        group_idx: col_logical / interpolate_degree,
        within_offset,
    }
}

/// Packs `collection` into the column-major, one-byte-per-element layout `YServer::new_rgswpir`
/// expects (`db[col * db_rows + row]`).
fn encode_db<T>(collection: &[T], params: &Params, interpolate_degree: usize) -> Vec<u16> {
    let entry_bytes = mem::size_of::<T>() as u64;
    let num_entries = collection.len() as u64;
    let gamma = params.poly_len;
    let db_rows = 1usize << (params.db_dim_1 + params.poly_len_log2);
    let db_cols = params.instances * gamma;

    let bytes: &[u8] = unsafe {
        slice::from_raw_parts(collection.as_ptr() as *const u8, (num_entries * entry_bytes) as usize)
    };

    let mut db = vec![0u16; db_rows * db_cols];
    for idx in 0..num_entries {
        let loc = locate(params, interpolate_degree, entry_bytes, idx);
        let col_start = (loc.group_idx * interpolate_degree + loc.sub_col) * gamma + loc.within_offset;
        let src = (idx * entry_bytes) as usize;
        for b in 0..entry_bytes as usize {
            db[(col_start + b) * db_rows + loc.row] = bytes[src + b] as u16;
        }
    }
    db
}

/// Server side: one insPIRe database for one level of a collection, rebuilt every round.
pub struct InspireServer {
    /// Leaked in `new` so `server` and `offline` can borrow it for `'static`; reclaimed in `Drop`.
    params: &'static Params,
    /// Encoded (interpolated) database. Leaked (not just `ManuallyDrop`) because
    /// `perform_offline_precomputation_simplepir`'s elided output lifetime is tied to `&self`, so
    /// producing a `'static`-bound `OfflinePrecomputedValues` requires calling it through a
    /// `'static` reference; reclaimed in `Drop`.
    server: &'static YServer<'static, u16>,
    /// Result of the offline (silent) preprocessing, reused by every query this round.
    offline: ManuallyDrop<OfflinePrecomputedValues<'static>>,
    /// Automorphism tables for the packing step; expensive to build, so computed once per shape.
    packing_params: PackParams<'static>,
    /// Records per row that the RGSW ciphertext selects between.
    interpolate_degree: usize,
    /// Number of Pung tuples in this database.
    num_entries: u64,
    /// Size of each Pung tuple in bytes.
    entry_bytes: u64,
}

impl InspireServer {
    /// Encodes `collection` and runs the offline precomputation (cpp_server_setup).
    pub fn new<T>(collection: &[T], _alpha: u64, _depth: u64) -> InspireServer {
        Self::new_with_breakdown(collection).0
    }

    /// Same as `new`, but also reports how long each phase of server setup took. Used by the
    /// `inspire_setup_breakdown` benchmark to separate shape-dependent work (parameter selection,
    /// packing tables) from data-dependent work (encoding, server construction, offline phase).
    pub fn new_with_breakdown<T>(collection: &[T]) -> (InspireServer, ServerSetupBreakdown) {
        let entry_bytes = mem::size_of::<T>() as u64;
        let num_entries = collection.len() as u64;

        // Shape-dependent: depends only on (num_entries, entry_bytes).
        let t = Instant::now();
        let dim0 = choose_dim0(num_entries, entry_bytes);
        let item_bits = (entry_bytes * 16) as usize;
        let (params, interpolate_degree, _) =
            params_rgswpir_given_input_size_and_dim0(num_entries as usize, item_bits, dim0);
        let params: &'static Params = Box::leak(Box::new(params));
        let t_params = t.elapsed();

        // Data-dependent: lay the tuples out in insPIRe's matrix format.
        let t = Instant::now();
        let db = encode_db(collection, params, interpolate_degree);
        let t_encode = t.elapsed();

        // Data-dependent: build insPIRe's server object over the encoded database.
        let t = Instant::now();
        let server = YServer::<u16>::new_rgswpir(params, &db, interpolate_degree);
        let server: &'static YServer<'static, u16> = Box::leak(Box::new(server));
        let t_yserver = t.elapsed();

        // Data-dependent: the offline (silent) preprocessing.
        let t = Instant::now();
        let offline = server.perform_offline_precomputation_simplepir(params.poly_len, None, false);
        let t_offline = t.elapsed();

        // Shape-dependent: automorphism tables for InspiRING packing.
        let t = Instant::now();
        let packing_params = PackParams::new(params, params.poly_len);
        let t_pack = t.elapsed();

        let breakdown = ServerSetupBreakdown {
            params: t_params,
            encode: t_encode,
            yserver: t_yserver,
            offline: t_offline,
            pack_params: t_pack,
            dim0,
            interpolate_degree,
        };

        let server = InspireServer {
            params,
            server,
            offline: ManuallyDrop::new(offline),
            packing_params,
            interpolate_degree,
            num_entries,
            entry_bytes,
        };
        (server, breakdown)
    }

    /// Answers one serialized query (cpp_server_process_query).
    pub fn gen_answer(&self, query: &[u8], _q_num: u64) -> InspireAnswer {
        let (mut packing_keys, packed_query_row, ct_gsw_body) =
            deserialize_everything(self.params, &self.packing_params, query.to_vec());

        let response = self.server.perform_online_computation_simplepir_and_rgsw(
            self.interpolate_degree,
            packed_query_row.as_slice(),
            ct_gsw_body,
            &self.offline,
            &mut packing_keys,
            None,
        );

        let num = response.len() as u64;
        InspireAnswer {
            answer: response.concat(),
            num,
        }
    }
}

impl Drop for InspireServer {
    /// Drops `offline` and `server`, then reclaims the leaked `params` (cpp_server_free).
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.offline);
            drop(Box::from_raw(self.server as *const YServer<'static, u16> as *mut YServer<'static, u16>));
            drop(Box::from_raw(self.params as *const Params as *mut Params));
        }
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
    /// Leaked so `y_client` can borrow it mutably for `'static`; reclaimed in `Drop`.
    client: *mut Client<'static>,
    /// Wraps the leaked spiral-rs `Client` holding the secret key.
    y_client: ManuallyDrop<YClient<'static>>,
    /// Automorphism tables for the packing step; expensive to build, so computed once per shape.
    packing_params: PackParams<'static>,
    interpolate_degree: usize,
    num_entries: u64,
    entry_bytes: u64,
    /// Index passed to the last `gen_query`, needed by `decode_answer` to extract the record.
    last_index: Option<u64>,
}

/// Builds a fresh client state for a database of `num_entries` entries of `entry_bytes` bytes each,
/// generating a new secret key (shared by `InspireClient::new` and `update_params`).
fn new_db_state(num_entries: u64, entry_bytes: u64) -> ClientDbState {
    new_db_state_with_breakdown(num_entries, entry_bytes).0
}

/// Same as `new_db_state`, but also reports how long each phase took.
fn new_db_state_with_breakdown(num_entries: u64, entry_bytes: u64) -> (ClientDbState, ClientSetupBreakdown) {
    let t = Instant::now();
    let dim0 = choose_dim0(num_entries, entry_bytes);
    let item_bits = (entry_bytes * 16) as usize;
    let (params, interpolate_degree, _) =
        params_rgswpir_given_input_size_and_dim0(num_entries as usize, item_bits, dim0);
    let params: &'static Params = Box::leak(Box::new(params));
    let t_params = t.elapsed();

    let t = Instant::now();
    let client_ptr: *mut Client<'static> = Box::into_raw(Box::new(Client::init(params)));
    let client_ref: &'static mut Client<'static> = unsafe { &mut *client_ptr };
    client_ref.generate_secret_keys();
    let t_keygen = t.elapsed();

    let t = Instant::now();
    let packing_params = PackParams::new(params, params.poly_len);
    let t_pack = t.elapsed();

    let t = Instant::now();
    let y_client = YClient::new(client_ref, params);
    let t_yclient = t.elapsed();

    let breakdown = ClientSetupBreakdown {
        params: t_params,
        keygen: t_keygen,
        pack_params: t_pack,
        yclient: t_yclient,
    };

    let state = ClientDbState {
        params,
        client: client_ptr,
        y_client: ManuallyDrop::new(y_client),
        packing_params,
        interpolate_degree,
        num_entries,
        entry_bytes,
        last_index: None,
    };
    (state, breakdown)
}

impl Drop for ClientDbState {
    /// Drops `y_client`, then reclaims the leaked `Client` and `params` (cpp_client_free).
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.y_client);
            drop(Box::from_raw(self.client));
            drop(Box::from_raw(self.params as *const Params as *mut Params));
        }
    }
}

impl InspireClient {
    /// Creates a client for a database of `num` entries of `size` bytes (cpp_client_setup).
    pub fn new(size: u64, num: u64, _alpha: u64, depth: u64) -> InspireClient {
        InspireClient {
            depth,
            db: RefCell::new(Some(new_db_state(num, size))),
        }
    }

    /// Same as `new`, but also reports how long each phase of client setup took.
    pub fn new_with_breakdown(size: u64, num: u64, depth: u64) -> (InspireClient, ClientSetupBreakdown) {
        let (state, breakdown) = new_db_state_with_breakdown(num, size);
        (
            InspireClient {
                depth,
                db: RefCell::new(Some(state)),
            },
            breakdown,
        )
    }

    /// Switches the client to a database of `num` entries of `size` bytes
    /// (cpp_client_update_db_params).
    pub fn update_params(&self, size: u64, num: u64, _alpha: u64) {
        *self.db.borrow_mut() = Some(new_db_state(num, size));
    }

    /// Builds the serialized query for entry `index` (cpp_client_generate_query).
    pub fn gen_query(&self, index: u64) -> InspireQuery {
        let mut db = self.db.borrow_mut();
        let state = db.as_mut().expect("update_params must be called before gen_query");
        state.last_index = Some(index);

        let params = state.params;
        let loc = locate(params, state.interpolate_degree, state.entry_bytes, index);

        let mut packing_keys = generate_packing_keys(
            params,
            &state.packing_params,
            state.y_client.client().get_sk_reg(),
            PackingType::InspiRING,
            params.poly_len,
        );
        let ct_gsw_body = generate_rgsw_query(&state.y_client, params, loc.sub_col, state.interpolate_degree);

        // `generate_query_over_prime` returns one ciphertext body element per database row
        // (length `db_rows`); there is no separate "mask" block to slice off here.
        let query_row =
            state
                .y_client
                .generate_query_over_prime(SEED_0, params.db_dim_1, PackingType::InspiRING, loc.row);
        let packed_query_row = pack_query(params, &query_row);

        InspireQuery {
            query: serialize_everything(params, &mut packing_keys, packed_query_row, ct_gsw_body),
            num: 1,
        }
    }

    /// Decrypts `answer` and extracts the entry requested by the last `gen_query`
    /// (cpp_client_process_reply).
    pub fn decode_answer(&self, answer: &[u8], a_num: u64) -> InspireResult {
        let db = self.db.borrow();
        let state = db.as_ref().expect("update_params must be called before decode_answer");
        let index = state.last_index.expect("gen_query must be called before decode_answer");

        let chunk_size = answer.len() / a_num as usize;
        let response: Vec<Vec<u8>> = answer.chunks_exact(chunk_size).map(|c| c.to_vec()).collect();

        let params = state.params;
        let decoded = decode_rgsw_response(&state.y_client, params, &response);

        let loc = locate(params, state.interpolate_degree, state.entry_bytes, index);
        let start = loc.group_idx * params.poly_len + loc.within_offset;
        let entry_bytes = state.entry_bytes as usize;

        InspireResult {
            result: decoded[start..start + entry_bytes].iter().map(|&v| v as u8).collect(),
        }
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



/// Time spent in each phase of `InspireServer::new`.
/// `params` and `pack_params` depend only on the database shape and could be cached across tree
/// levels, buckets and rounds; `encode`, `yserver` and `offline` depend on the database contents.
pub struct ServerSetupBreakdown {
    pub params: Duration,
    pub encode: Duration,
    pub yserver: Duration,
    pub offline: Duration,
    pub pack_params: Duration,
    pub dim0: usize,
    pub interpolate_degree: usize,
}

/// Time spent in each phase of client setup (`InspireClient::new` / `update_params`).
pub struct ClientSetupBreakdown {
    pub params: Duration,
    pub keygen: Duration,
    pub pack_params: Duration,
    pub yclient: Duration,
}
