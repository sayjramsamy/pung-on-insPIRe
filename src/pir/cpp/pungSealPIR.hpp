#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Server lifecycle
void* cpp_sealpir_server_setup(uint64_t num_db_entries, uint64_t entry_size_bytes);
void  cpp_sealpir_server_free(void* server);

// Client lifecycle
void* cpp_sealpir_client_setup(uint64_t num_db_entries, uint64_t entry_size_bytes);
void  cpp_sealpir_client_free(void* client);

// Buffer free (for Drop impls in Rust)
void cpp_sealpir_buffer_free(char* buf);

// Query generation (client)
// Returns serialized query bytes; sets *rlen_query_bytes
char* cpp_sealpir_client_generate_query(void* client, uint64_t chosen_idx, uint64_t* rlen_query_bytes);

// Answer generation (server)
// Returns serialized answer bytes; sets *rlen_answer_bytes
char* cpp_sealpir_server_process_query(void* server, const char* db, char* query, uint64_t len_query_bytes, uint64_t* rlen_answer_bytes);

// Answer decoding (client)
// Returns the plaintext entry bytes; sets *rlen_result_bytes
char* cpp_sealpir_client_process_reply(void* client, char* answer, uint64_t len_answer_bytes, uint64_t* rlen_result_bytes);

// Key exchange: client generates galois keys, server stores them
char* cpp_sealpir_client_get_galois_keys(void* client, uint64_t* rlen_keys_bytes);
void  cpp_sealpir_server_set_galois_keys(void* server, const char* keys, uint64_t len_keys_bytes);

#ifdef __cplusplus
}
#endif
