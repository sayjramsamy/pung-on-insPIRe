use libc::c_char;
use std::os::raw::c_void;
use std::slice;

use super::sealpir_server::{SealPirAnswer, SealPirQuery, SealPirResult};

extern "C" {
    fn cpp_sealpir_client_setup(num_db_entries: u64, entry_size_bytes: u64) -> *mut c_void;
    fn cpp_sealpir_client_free(client: *mut c_void);
    fn cpp_sealpir_buffer_free(buf: *mut c_char);

    fn cpp_sealpir_client_generate_query(
        client: *mut c_void,
        chosen_idx: u64,
        rlen_query_bytes: *mut u64,
    ) -> *mut c_char;

    fn cpp_sealpir_client_process_reply(
        client: *mut c_void,
        answer: *mut c_char,
        len_answer_bytes: u64,
        rlen_result_bytes: *mut u64,
    ) -> *mut c_char;

    fn cpp_sealpir_client_get_galois_keys(client: *mut c_void, rlen_keys_bytes: *mut u64) -> *mut c_char;
}

pub struct SealPirClient {
    client: *mut c_void,
    #[allow(dead_code)]
    num_entries: u64,
    #[allow(dead_code)]
    entry_size: u64,
}

impl Drop for SealPirClient {
    fn drop(&mut self) {
        unsafe {
            cpp_sealpir_client_free(self.client);
        }
    }
}

impl SealPirClient {
    pub fn new(num_entries: u64, entry_size: u64) -> SealPirClient {
        let client = unsafe { cpp_sealpir_client_setup(num_entries, entry_size) };

        SealPirClient {
            client,
            num_entries,
            entry_size,
        }
    }

    pub fn get_galois_keys(&self) -> Vec<u8> {
        let mut len: u64 = 0;

        unsafe {
            let ptr = cpp_sealpir_client_get_galois_keys(self.client, &mut len);
            let bytes = slice::from_raw_parts(ptr as *const u8, len as usize).to_vec();
            cpp_sealpir_buffer_free(ptr);
            bytes
        }
    }

    pub fn gen_query<'a>(&self, index: u64) -> SealPirQuery<'a> {
        let mut q_len: u64 = 0;

        let query_ptr = unsafe {
            cpp_sealpir_client_generate_query(self.client, index, &mut q_len)
        };

        SealPirQuery::new(query_ptr, q_len as usize)
    }

    pub fn decode_answer<'a>(&self, answer: SealPirAnswer<'a>) -> SealPirResult<'a> {
        let mut r_len: u64 = 0;

        let result_ptr = unsafe {
            cpp_sealpir_client_process_reply(
                self.client,
                answer.answer.0,
                answer.answer.1 as u64,
                &mut r_len,
            )
        };

        SealPirResult::new(result_ptr, r_len as usize)
    }

    // Takes a borrowed, serialized answer buffer (e.g. bytes received over the
    // network) instead of an owned SealPirAnswer. cpp_sealpir_client_process_reply
    // only reads from `answer`, so no ownership/free hazard here.
    pub fn decode_answer_from_bytes<'a>(&self, answer_bytes: &[u8]) -> SealPirResult<'a> {
        let mut r_len: u64 = 0;

        let result_ptr = unsafe {
            cpp_sealpir_client_process_reply(
                self.client,
                answer_bytes.as_ptr() as *mut c_char,
                answer_bytes.len() as u64,
                &mut r_len,
            )
        };

        SealPirResult::new(result_ptr, r_len as usize)
    }
}
