# XPIR PIR API Surface in Pung

Task 1b: what Pung uses from XPIR, and therefore what an insPIRe backend must replicate.
Based on `src/pir/cpp/pungPIR.hpp`, `src/pir/cpp/pungPIR.cpp`, the Rust bindings in `src/pir/`, their
call sites, `schema/pung.capnp` and `build.rs` (repo at commit `d5dfa97`).

## Summary

Pung's Rust code never calls XPIR directly. It only uses a C interface of **9 functions** exported by
`src/pir/cpp/pungPIR.cpp`. Everything above that interface (the Rust `PirServer` / `PirClient` API and
its callers) is backend-independent. Because insPIRe is itself a Rust crate, the planned integration
implements that Rust API directly rather than the C ABI; see "Recommended integration plan" at the end.

```
Pung callers (db/mod.rs, server/rpc.rs, client/mod.rs)
        │
Rust wrappers (src/pir/: PirServer, PirClient, PirQuery/PirAnswer/PirResult)
        │
C ABI: 9 extern "C" functions   ◄── XPIR's contract
        │
        └── pungPIR.cpp + deps/xpir        (XPIR, today)

insPIRe (planned) plugs in at the Rust-wrapper level instead, as a Rust implementation of src/pir/.
```

## Layer 1: Rust API (what Pung actually uses)

| Rust call | Call site | C function |
|---|---|---|
| `PirServer::new(&[PungTuple], alpha, depth)` | `src/db/mod.rs:600` (one per level, collection and bucket) | `cpp_server_setup` |
| `server.gen_answer(query, q_num) -> PirAnswer` | `src/server/rpc.rs:523` | `cpp_server_process_query` |
| `PirClient::new(1, 1, 1, depth)` (placeholder values) | `src/client/mod.rs:199` | `cpp_client_setup` |
| `client.update_params(TUPLE_SIZE, len, alpha)` | `src/client/mod.rs:1682`, before every query | `cpp_client_update_db_params` |
| `client.gen_query(idx) -> PirQuery` | `src/client/mod.rs:1685` | `cpp_client_generate_query` |
| `client.decode_answer(answer, a_num) -> PirResult` | `src/client/mod.rs:1709` | `cpp_client_process_reply` |
| `Drop` on server / client / buffers | `src/pir/*.rs` | `cpp_server_free`, `cpp_client_free`, `cpp_buffer_free` |

Data structures (`src/pir/mod.rs`):

- `PirQuery { query: &mut [u8], num: u64 }`
- `PirAnswer { answer: &mut [u8], num: u64 }`
- `PirResult { result: &mut [u8] }`

Each wraps a C-allocated buffer and frees it on `Drop` via `cpp_buffer_free`.

On the network, a query and an answer are each a byte blob plus a count (`schema/pung.capnp:16`):
`retr(..., query :Data, qnum :UInt64) -> (answer :Data, anum :UInt64)`.

## Layer 2: C ABI (the contract insPIRe must meet)

```c
void* cpp_server_setup(uint64_t len_total_bytes, char* db, uint64_t num_entries, uint64_t alpha, uint64_t d);
char* cpp_server_process_query(void* server, char* q, uint64_t q_len, uint64_t q_num,
                               uint64_t* a_len, uint64_t* a_num);
void  cpp_server_free(void* server);

void* cpp_client_setup(uint64_t len_total_bytes, uint64_t num_entries, uint64_t alpha, uint64_t d);
void  cpp_client_update_db_params(void* client, uint64_t len_total_bytes, uint64_t num_entries,
                                  uint64_t alpha, uint64_t d);
char* cpp_client_generate_query(void* client, uint64_t idx, uint64_t* q_len, uint64_t* q_num);
char* cpp_client_process_reply(void* client, char* a, uint64_t a_len, uint64_t a_num, uint64_t* r_len);
void  cpp_client_free(void* client);

void  cpp_buffer_free(char* buf);
```

Rules the code relies on but never states:

1. **Records are fixed-size.** Each record is `len_total_bytes / num_entries` bytes. In Pung that is
   `TUPLE_SIZE` = 32 label + 238 cipher + 16 MAC = **286 bytes** (`src/db/mod.rs`). `decode_answer` must
   return exactly those 286 bytes for the requested index.
2. **Buffers are allocated with `malloc`.** Every returned buffer is freed with `free()` through
   `cpp_buffer_free`, so outputs must be `malloc`/`calloc`'d, not created with `new[]` or owned by Rust.
3. **Queries and answers are packed elements.** Each is `num` equal-sized elements laid end to end;
   element size = `len / num`. XPIR needs this because a query is a vector of LWE ciphertexts. insPIRe can
   simply return `num = 1` with an opaque blob.
4. **The server keeps its own copy of the DB.** `cpp_server_setup` frees the padded DB right after
   construction (`pungPIR.cpp:291`), so the server must not keep a pointer to the caller's buffer.
5. **The client keeps state between calls.** It stores `lastChosenIdx` between `gen_query` and
   `decode_answer`. One client object, including its crypto state and secret key, is reused across every
   DB it queries; `update_params` only changes the DB shape.
6. **There is no error channel.** Failures call `exit(1)`; Rust assumes returned pointers are valid.

## Layer 3: XPIR internals wrapped by the shim (what insPIRe replaces)

- **`PIRParameters`**: `alpha`, `d`, `n[0..2]`, `crypto_params`. `crypto_params` is hard-coded to
  `"LWE:80:1024:60"` (**80-bit security**; relevant when comparing benchmarks).
- **`HomomorphicCryptoFactory::getCryptoMethod(crypto_params)`** returns a `HomomorphicCrypto*`;
  `setandgetAbsBitPerCiphertext(n[0])` is called on both sides.
- **Server side:**
  - `DBArrayProcessor(len, buf, n)`: a Pung addition to XPIR that reads the DB from memory instead of a
    file; `getmaxFileBytesize()`.
  - `PIRReplyGenerator(params, crypto, db)`: `setPirParams`, `importData(0, size)` returns an
    `imported_database*` (done once at setup), then per query `pushQuery`, `generateReply(imported_db)`,
    `popReply`, `getReplyElementBytesize`, `freeQueries`.
- **Client side:**
  - `PIRQueryGenerator(params, crypto)`: `generateQuery(idx / alpha)`, `popQuery`,
    `getQueryElementBytesize`.
  - `PIRReplyExtraction(params, crypto)`: `pushEncryptedReply`, `extractReply(size)`,
    `popPlaintextResult`, `getPlaintextReplyBytesize`, `getnbPlaintextReplies`.

## Pung-specific logic in the shim

- **`alpha` (aggregation):** `alpha` records are packed into each PIR element. The server pads the DB with
  `0x01` bytes to a multiple of `alpha`; the client queries `idx / alpha` and then slices out record
  `idx % alpha` from the decoded element. The alpha values in `util::get_alpha` (`src/util/mod.rs:135`)
  were tuned for XPIR; for insPIRe, pass `alpha = 1` or re-tune.
- **`d` (recursion depth, 1 or 2):** for `d = 2` the shim factors `n / alpha` into an m×n layout
  (`DefaultPIRParams::Factorize`). This is XPIR-specific; insPIRe can accept and ignore it. It comes from
  the `-d` command-line flag of the client and server binaries.

## Implications for insPIRe

These were written before reading the insPIRe code; see "insPIRe implementation" below for what the
code actually does.

- **Server setup runs every round.** `db.pir_setup()` is called on every round
  (`src/server/send_dataflow.rs:85`), rebuilding a `PirServer` for every level of every collection. Any
  insPIRe preprocessing therefore sits on the per-round critical path and should be reported separately in
  benchmarks.
- **There is no key-setup step.** XPIR's client keys never leave the client, so the API has no RPC for
  uploading keys. If insPIRe needs per-client public or evaluation keys, either put them in every query
  (inflating upload size) or add a new RPC plus Rust changes. A CRS can be derived from a fixed seed shared
  by both sides.
- **The client never sees the DB.** It knows only `(num_entries, 286)`. Any client-side hint insPIRe needs
  must come through a new channel.

## Test harness and build wiring

- `tests/pir.rs` and `benches/pir.rs` exercise exactly the Rust API above, so they serve as a correctness
  check and a micro-benchmark for both backends.
- `build.rs` compiles the shim against `deps/xpir`, builds XPIR with CMake, and links `gomp`, `gmp`,
  `mpfr` and `boost_thread`. This is where to add a switch (Cargo feature or environment variable) between
  XPIR and insPIRe.

## insPIRe implementation: API and mapping to Pung

Source: [google/private-membership](https://github.com/google/private-membership), `research/InsPIRe`,
commit `25b2fa3`, cloned to `~/private-membership`. The repo cites the paper as *"InsPIRe:
Communication-Efficient PIR with Silent Preprocessing"*, [ePrint 2025/1352](https://eprint.iacr.org/2025/1352).

The crate is Rust (`inspire`, edition 2021, pinned to `nightly-2024-02-07`, depends on `spiral-rs` at git
rev `6929441`). It has four protocol variants; the paper's main **InsPIRe** scheme is the one run by
`cargo run --release --bin inspire -- --num-items N --item-size-bits B --dim0 D`.

### Hardware requirement: AVX-512

The hot kernels (`src/kernel.rs` `fast_batched_dot_product_avx512*`, `src/packing.rs:736`) call `_mm512_*`
intrinsics with no fallback and no runtime CPU detection, and the build uses `-C target-cpu=native` /
`-march=native`. The development laptop (AMD Ryzen 5 4500U, Zen 2) has no AVX-512, so insPIRe cannot be
benchmarked on it. Benchmarks need an AVX-512 machine (Intel Ice Lake+ or AMD Zen 4+), and the XPIR
baseline in `results/` must be re-run on that same machine for a fair comparison.

### Where the main scheme lives

`src/client.rs` and `src/server.rs` are library building blocks (shared with the YPIR-derived
SimplePIR / DoublePIR / InsPIRe⁽²⁾ variants). The main InsPIRe protocol is assembled in the **binary**
`src/bin/inspire.rs`, so these parts must be moved into a library module before Pung can call them:

- trait `RGSWPIR`: `YServer::<u16>::new_rgswpir(&params, &[u16], interpolate_degree)` and
  `perform_online_computation_simplepir_and_rgsw(...)`
- `params_rgswpir_given_input_size_and_dim0(num_items, item_size_bits, dim0) -> (Params, interpolate_degree, dims)`
- `serialize_everything` / `deserialize_everything` (query to/from bytes)
- the RGSW query encryption (an inline `t_gsw` loop in `run_simple_ypir_rgsw_on_params`)

### Public library API used by the main scheme

| Item | File | Purpose |
|---|---|---|
| `YClient<'a>`, `YClient::new(&'a mut Client<'a>, &'a Params)` | `client.rs` | Client wrapper; borrows the spiral-rs `Client` (secret key) and `Params` |
| `generate_query_over_prime(seed_idx, dim_log2, PackingType, row) -> Vec<u64>` | `client.rs` | Row-selection query (first dimension) |
| `pack_query(&Params, &[u64]) -> AlignedMemory64` | `client.rs` | Packs the row query for the server |
| `decrypt_ct_reg_measured(...)` | `client.rs` | Decrypts one response ciphertext |
| `YServer<'a, T>` | `server.rs` | Encoded DB (`db_buf_aligned`), borrowed `&'a Params`, packing parameters |
| `perform_offline_precomputation_simplepir(gamma, None, online_only) -> OfflinePrecomputedValues` | `server.rs` | Offline phase; depends on the DB and public seeds only |
| `PackingKeys::init_full(&PackParams, sk, W_SEED, V_SEED)` | `packing.rs` | Client packing keys, derived from the secret key |

`YClient::decode_response` and `YServer::new` / `perform_online_computation{,_medium_payload}` belong to the
other variants and are not used by the main scheme.

### Flow

```
BOTH    params, d = params_rgswpir_given_input_size_and_dim0(num_items, item_bits, dim0)   // deterministic

SERVER  server  = YServer::<u16>::new_rgswpir(&params, &db_u16, d)
setup   offline = server.perform_offline_precomputation_simplepir(gamma = poly_len, None, false)

CLIENT  client = Client::init(&params); client.generate_secret_keys()
query   keys   = PackingKeys::init_full(&PackParams::new(&params, gamma), sk, W_SEED, V_SEED)
        ct_gsw = RGSW encryption of X^(sub-column)                  // selects the column within a row
        q_row  = pack_query(&params, &y_client.generate_query_over_prime(SEED_0, db_dim_1, InspiRING, row))
        bytes  = serialize_everything(&params, &mut keys, q_row, ct_gsw)

SERVER  (keys, q_row, ct_gsw) = deserialize_everything(&params, bytes)
answer  resp = server.perform_online_computation_simplepir_and_rgsw(d, &q_row, ct_gsw, &offline, &mut keys, None)
                                                                    // Vec<Vec<u8>>: c ciphertexts

CLIENT  per ciphertext: PolyMatrixRaw::recover_how_many(...) -> decrypt_ct_reg_measured(...)[..gamma]
decode  concatenation = the record as c·gamma u16 plaintext elements
```

The CRS is a set of hard-coded constants shared by both sides (`STATIC_PUBLIC_SEED`, `W_SEED`, `V_SEED`,
`RGSW_SEEDS`), so Pung needs no setup exchange for it.

### Mapping onto Pung's PIR interface

| Pung (Layer 1 / 2) | insPIRe |
|---|---|
| `PirServer::new` / `cpp_server_setup` | params + `new_rgswpir` + offline precomputation |
| `PirClient::new`, `update_params` | `Client::init`, secret keys, packing keys; params recomputed from `(num, size)` |
| `gen_query` | row query + RGSW ciphertext + packing keys, serialized to one byte blob (`num = 1`) |
| `gen_answer` | deserialize + `perform_online_computation_simplepir_and_rgsw`; concatenate the `c` ciphertexts |
| `decode_answer` | `recover_how_many` + decrypt; convert elements back to bytes; extract the record |

Like XPIR, the client is stateful: the secret key must persist between `gen_query` and `decode_answer`.

### Entry size: a runtime parameter, with constraints

- `item_size_bits` and `num_items` are runtime inputs to the params function, which returns a runtime
  `Params`. The only compile-time choice is the plaintext element type (`u16` for InsPIRe).
- Hard-coded in the params function: `poly_len = 2048`, plaintext modulus `p = 65535`, `q2_bits = 28`,
  `t_exp = 3`.
- **Granularity is one polynomial**, 2048 × 16 bits = 4 KiB. Smaller records are packed
  `factor = ⌊32768 / item_bits⌋` to a polynomial: 14 Pung tuples of 286 B each. This plays the role of
  XPIR's `alpha`.
- **The library does no byte encoding or record extraction.** The benchmark binary fills the DB with random
  `u16`s and checks raw elements. The Pung wrapper must implement:
  - bytes → `u16` elements, laid out column-major (`db[col * db_rows + row]`);
  - record index → (row, sub-column) mapping;
  - extracting the record from the `c·gamma` decoded elements.
- **Elements must be < 65535.** Values are reduced mod 65535, not 65536, so raw 16-bit words cannot be
  packed (`0xFFFF` decodes as `0`). Pack 15 bits per element, or handle that value specially.

### Integration friction

1. **Lifetimes:** `YServer<'a>`, `YClient<'a>`, `PackingKeys<'a>` and `OfflinePrecomputedValues<'a>` all borrow
   `&'a Params`, and `YClient` also borrows `&'a mut Client`. Storing them in Pung's long-lived
   `PirServer` / `PirClient` needs an owning wrapper (boxed or leaked `Params`, or a self-referential-struct
   crate).
2. **Keys are uploaded with every query:** the packing keys (`3 × 2048 × 2` u64 ≈ 96 KiB in
   `serialize_everything`, before the query row and RGSW ciphertext) are part of each serialized query.
   Pung has no key channel, so the simplest integration sends them every time.
3. **Per-round setup:** the offline phase needs no client input, so it fits Pung's server-only
   `pir_setup()`. Pung calls that every round, so encode + offline time sits on the per-round critical
   path and should be reported separately.
4. **Toolchain:** insPIRe pins `nightly-2024-02-07`; Pung is a 2015-edition crate built on stable 1.98 with
   `RUSTC_BOOTSTRAP=1`. Both must build under one toolchain.

## Recommended integration plan

insPIRe is a Rust crate, so a C++ shim is unnecessary. Add `inspire` as a Cargo dependency of Pung, move
the main-scheme code out of `src/bin/inspire.rs` into a library module, and implement the `PirServer` /
`PirClient` API of `src/pir/` directly in Rust behind a backend switch (a Cargo feature). XPIR and insPIRe
runs then share identical calling code, and `tests/pir.rs` / `benches/pir.rs` work unchanged.
