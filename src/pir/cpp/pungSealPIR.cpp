// Shim layer that exposes Microsoft SealPIR's PIRServer/PIRClient through the
// extern "C" interface declared in pungSealPIR.hpp, so Rust can drive it the
// same way it drives the XPIR backend in pungPIR.cpp.

#include "pungSealPIR.hpp"
#include "pir.hpp"
#include "pir_client.hpp"
#include "pir_server.hpp"

#include <cstring>
#include <memory>
#include <sstream>
#include <string>

using namespace std;
using namespace seal;

namespace {

// SealPIR's own recommended parameters for (N, logt, d): see SealPIR's
// src/main.cpp. These must match between client and server.
constexpr uint32_t kPolyModulusDegree = 4096;
constexpr uint32_t kLogT = 20;
constexpr uint32_t kDimensions = 2;
constexpr uint32_t kClientId = 0;

void gen_params(uint64_t num_db_entries, uint64_t entry_size_bytes,
                 EncryptionParameters &enc_params, PirParams &pir_params) {
  enc_params = EncryptionParameters(scheme_type::bfv);
  gen_encryption_params(kPolyModulusDegree, kLogT, enc_params);
  verify_encryption_params(enc_params);
  gen_pir_params(num_db_entries, entry_size_bytes, kDimensions, enc_params,
                 pir_params, /*enable_symmetric=*/true,
                 /*enable_batching=*/true, /*enable_mswitching=*/true);
}

char *string_to_c_buffer(const string &data, uint64_t *rlen) {
  *rlen = data.size();
  char *buf = new char[data.size()];
  memcpy(buf, data.data(), data.size());
  return buf;
}

} // namespace

struct SealPirServerState {
  EncryptionParameters enc_params;
  PirParams pir_params;
  unique_ptr<PIRServer> server;
  uint64_t ele_num;
  uint64_t ele_size;
  bool db_set;
};

struct SealPirClientState {
  EncryptionParameters enc_params;
  PirParams pir_params;
  unique_ptr<PIRClient> client;
  uint64_t last_offset;
};

extern "C" {

void *cpp_sealpir_server_setup(uint64_t num_db_entries,
                                uint64_t entry_size_bytes) {
  SealPirServerState *state = new SealPirServerState();
  gen_params(num_db_entries, entry_size_bytes, state->enc_params,
             state->pir_params);
  state->server = make_unique<PIRServer>(state->enc_params, state->pir_params);
  state->ele_num = num_db_entries;
  state->ele_size = entry_size_bytes;
  state->db_set = false;
  return state;
}

void cpp_sealpir_server_free(void *server) {
  delete static_cast<SealPirServerState *>(server);
}

void *cpp_sealpir_client_setup(uint64_t num_db_entries,
                                uint64_t entry_size_bytes) {
  SealPirClientState *state = new SealPirClientState();
  gen_params(num_db_entries, entry_size_bytes, state->enc_params,
             state->pir_params);
  state->client = make_unique<PIRClient>(state->enc_params, state->pir_params);
  state->last_offset = 0;
  return state;
}

void cpp_sealpir_client_free(void *client) {
  delete static_cast<SealPirClientState *>(client);
}

void cpp_sealpir_buffer_free(char *buf) { delete[] buf; }

char *cpp_sealpir_client_generate_query(void *client_ptr, uint64_t chosen_idx,
                                         uint64_t *rlen_query_bytes) {
  SealPirClientState *state = static_cast<SealPirClientState *>(client_ptr);

  uint64_t fv_index = state->client->get_fv_index(chosen_idx);
  state->last_offset = state->client->get_fv_offset(chosen_idx);

  stringstream out_stream;
  state->client->generate_serialized_query(fv_index, out_stream);
  return string_to_c_buffer(out_stream.str(), rlen_query_bytes);
}

char *cpp_sealpir_server_process_query(void *server_ptr, const char *db,
                                        char *query, uint64_t len_query_bytes,
                                        uint64_t *rlen_answer_bytes) {
  SealPirServerState *state = static_cast<SealPirServerState *>(server_ptr);

  if (!state->db_set) {
    uint64_t total_bytes = state->ele_num * state->ele_size;
    uint8_t *raw = new uint8_t[total_bytes];
    memcpy(raw, db, total_bytes);
    unique_ptr<const uint8_t[]> db_bytes(raw);
    state->server->set_database(db_bytes, state->ele_num, state->ele_size);
    state->server->preprocess_database();
    state->db_set = true;
  }

  stringstream in_stream(string(query, len_query_bytes));
  PirQuery pir_query = state->server->deserialize_query(in_stream);

  PirReply reply = state->server->generate_reply(pir_query, kClientId);

  stringstream out_stream;
  state->server->serialize_reply(reply, out_stream);
  return string_to_c_buffer(out_stream.str(), rlen_answer_bytes);
}

char *cpp_sealpir_client_process_reply(void *client_ptr, char *answer,
                                        uint64_t len_answer_bytes,
                                        uint64_t *rlen_result_bytes) {
  SealPirClientState *state = static_cast<SealPirClientState *>(client_ptr);

  // generate_reply's final ciphertext count depends on d: it's 1 for d=1,
  // but expansion_ratio^(d-1) in general (PIRServer::generate_reply grows
  // `product` by expansion_ratio once per intermediate dimension via
  // `product = intermediate_plain.size();`). serialize_reply just writes all
  // of them back-to-back with no count prefix, so read until the buffer is
  // exhausted rather than hardcoding a ciphertext count here.
  SEALContext context(state->enc_params, true);
  stringstream in_stream(string(answer, len_answer_bytes));

  PirReply reply;
  while (in_stream.peek() != std::char_traits<char>::eof()) {
    Ciphertext ct;
    ct.load(context, in_stream);
    reply.push_back(ct);
  }

  vector<uint8_t> result =
      state->client->decode_reply(reply, state->last_offset);

  *rlen_result_bytes = result.size();
  char *buf = new char[result.size()];
  memcpy(buf, result.data(), result.size());
  return buf;
}

char *cpp_sealpir_client_get_galois_keys(void *client_ptr,
                                          uint64_t *rlen_keys_bytes) {
  SealPirClientState *state = static_cast<SealPirClientState *>(client_ptr);

  GaloisKeys galois_keys = state->client->generate_galois_keys();
  stringstream out_stream;
  galois_keys.save(out_stream);
  return string_to_c_buffer(out_stream.str(), rlen_keys_bytes);
}

void cpp_sealpir_server_set_galois_keys(void *server_ptr, const char *keys,
                                         uint64_t len_keys_bytes) {
  SealPirServerState *state = static_cast<SealPirServerState *>(server_ptr);

  SEALContext context(state->enc_params, true);
  GaloisKeys galois_keys;
  stringstream in_stream(string(keys, len_keys_bytes));
  galois_keys.load(context, in_stream);

  state->server->set_galois_key(kClientId, galois_keys);
}

} // extern "C"
