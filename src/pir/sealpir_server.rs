use libc::c_char;
use std::os::raw::c_void;
use std::slice;

extern "C" {
    fn cpp_sealpir_server_setup(num_db_entries: u64, entry_size_bytes: u64) -> *mut c_void;
    fn cpp_sealpir_server_free(server: *mut c_void);
    fn cpp_sealpir_buffer_free(buf: *mut c_char);

    fn cpp_sealpir_server_process_query(
        server: *mut c_void,
        db: *const c_char,
        query: *mut c_char,
        len_query_bytes: u64,
        rlen_answer_bytes: *mut u64,
    ) -> *mut c_char;

    fn cpp_sealpir_server_set_galois_keys(server: *mut c_void, keys: *const c_char, len_keys_bytes: u64);
}

pub struct SealPirQuery<'a> {
    pub query: (*mut c_char, usize),
    _marker: std::marker::PhantomData<&'a ()>,
}

pub struct SealPirAnswer<'a> {
    pub answer: (*mut c_char, usize),
    _marker: std::marker::PhantomData<&'a ()>,
}

pub struct SealPirResult<'a> {
    pub result: (*mut c_char, usize),
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Drop for SealPirQuery<'a> {
    fn drop(&mut self) {
        unsafe {
            cpp_sealpir_buffer_free(self.query.0);
        }
    }
}

impl<'a> Drop for SealPirAnswer<'a> {
    fn drop(&mut self) {
        unsafe {
            cpp_sealpir_buffer_free(self.answer.0);
        }
    }
}

impl<'a> Drop for SealPirResult<'a> {
    fn drop(&mut self) {
        unsafe {
            cpp_sealpir_buffer_free(self.result.0);
        }
    }
}

impl<'a> SealPirQuery<'a> {
    pub fn new(ptr: *mut c_char, len: usize) -> SealPirQuery<'a> {
        SealPirQuery {
            query: (ptr, len),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        unsafe { slice::from_raw_parts(self.query.0 as *const u8, self.query.1).to_vec() }
    }
}

impl<'a> SealPirAnswer<'a> {
    pub fn new(ptr: *mut c_char, len: usize) -> SealPirAnswer<'a> {
        SealPirAnswer {
            answer: (ptr, len),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        unsafe { slice::from_raw_parts(self.answer.0 as *const u8, self.answer.1).to_vec() }
    }
}

impl<'a> SealPirResult<'a> {
    pub fn new(ptr: *mut c_char, len: usize) -> SealPirResult<'a> {
        SealPirResult {
            result: (ptr, len),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        unsafe { slice::from_raw_parts(self.result.0 as *const u8, self.result.1).to_vec() }
    }
}

pub struct SealPirServer<'a> {
    ptr: *mut c_void,
    db: &'a [u8],
    #[allow(dead_code)]
    num_entries: u64,
    #[allow(dead_code)]
    entry_size: u64,
}

impl<'a> Drop for SealPirServer<'a> {
    fn drop(&mut self) {
        unsafe {
            cpp_sealpir_server_free(self.ptr);
        }
    }
}

impl<'a> SealPirServer<'a> {
    pub fn new(db: &'a [u8], num_entries: u64, entry_size: u64) -> SealPirServer<'a> {
        let ptr = unsafe { cpp_sealpir_server_setup(num_entries, entry_size) };

        SealPirServer {
            ptr,
            db,
            num_entries,
            entry_size,
        }
    }

    pub fn set_galois_keys(&self, keys: &[u8]) {
        unsafe {
            cpp_sealpir_server_set_galois_keys(
                self.ptr,
                keys.as_ptr() as *const c_char,
                keys.len() as u64,
            );
        }
    }

    pub fn gen_answer(&self, query: &SealPirQuery) -> SealPirAnswer<'a> {
        let mut a_len: u64 = 0;

        let answer_ptr = unsafe {
            cpp_sealpir_server_process_query(
                self.ptr,
                self.db.as_ptr() as *const c_char,
                query.query.0,
                query.query.1 as u64,
                &mut a_len,
            )
        };

        SealPirAnswer::new(answer_ptr, a_len as usize)
    }

    // Takes a borrowed, serialized query buffer (e.g. bytes received over the
    // network) instead of an owned SealPirQuery. cpp_sealpir_server_process_query
    // only reads from `query`, so no ownership/free hazard here.
    pub fn gen_answer_from_bytes(&self, query_bytes: &[u8]) -> SealPirAnswer<'a> {
        let mut a_len: u64 = 0;

        let answer_ptr = unsafe {
            cpp_sealpir_server_process_query(
                self.ptr,
                self.db.as_ptr() as *const c_char,
                query_bytes.as_ptr() as *mut c_char,
                query_bytes.len() as u64,
                &mut a_len,
            )
        };

        SealPirAnswer::new(answer_ptr, a_len as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::super::sealpir_client::SealPirClient;
    use super::*;

    #[test]
    fn sealpir_roundtrip() {
        const NUM_ENTRIES: u64 = 64;
        const ENTRY_SIZE: u64 = 256;
        const CHOSEN_IDX: u64 = 42;

        let mut db = vec![0u8; (NUM_ENTRIES * ENTRY_SIZE) as usize];
        let pattern: Vec<u8> = (0..ENTRY_SIZE as usize).map(|i| (i % 256) as u8).collect();
        let start = (CHOSEN_IDX * ENTRY_SIZE) as usize;
        db[start..start + ENTRY_SIZE as usize].copy_from_slice(&pattern);

        let client = SealPirClient::new(NUM_ENTRIES, ENTRY_SIZE);
        let server = SealPirServer::new(&db, NUM_ENTRIES, ENTRY_SIZE);

        let galois_keys = client.get_galois_keys();
        server.set_galois_keys(&galois_keys);

        let query = client.gen_query(CHOSEN_IDX);
        let answer = server.gen_answer(&query);
        let result = client.decode_answer(answer);

        assert_eq!(result.to_bytes(), pattern);
    }
}
