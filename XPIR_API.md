# XPIR PIR API Surface in Pung

Task 1b: what Pung uses from XPIR, and therefore what an insPIRe backend must replicate.
Based on `src/pir/cpp/pungPIR.hpp`, `src/pir/cpp/pungPIR.cpp`, the Rust bindings in `src/pir/`, their
call sites, `schema/pung.capnp` and `build.rs` (repo at commit `d5dfa97`).

## Summary

Pung's Rust code never calls XPIR directly. It only uses a C interface of **9 functions** exported by
`src/pir/cpp/pungPIR.cpp`. An insPIRe backend that exports the same 9 functions and follows the contract
rules below can replace XPIR with **no change to Pung's Rust code**, so both backends are benchmarked on
identical code.

```
Pung callers (db/mod.rs, server/rpc.rs, client/mod.rs)
        │
Rust wrappers (src/pir/: PirServer, PirClient, PirQuery/PirAnswer/PirResult)
        │
C ABI: 9 extern "C" functions   ◄── the contract to keep
        │
        ├── pungPIR.cpp + deps/xpir        (today)
        └── new shim + insPIRe             (planned)
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

## Implications for insPIRe (check against the paper)

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

## Recommended integration plan

Write a new shim (e.g. `src/pir/cpp/pungInsPIRe.cpp`) that exports the same 9 functions, and choose the
backend in `build.rs`. XPIR and insPIRe runs then share identical Rust code, and `tests/pir.rs` /
`benches/pir.rs` work unchanged.
