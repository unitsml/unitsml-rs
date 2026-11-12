use crate::{evaluate_json, wire::refusal};
use std::{
    ffi::{c_char, CString},
    panic::catch_unwind,
    ptr, slice,
};

#[no_mangle]
pub extern "C" fn unitsml_abi_version() -> u32 {
    1
}

#[no_mangle]
pub unsafe extern "C" fn unitsml_evaluate_json_v1(input: *const u8, length: usize) -> *mut c_char {
    let result = catch_unwind(|| {
        let response = if length > 1024 * 1024 {
            refusal("error", "resource_limit", "request exceeds 1 MiB")
        } else if input.is_null() && length != 0 {
            refusal("error", "invalid_request", "null input with nonzero length")
        } else {
            let bytes = if length == 0 {
                &[]
            } else {
                unsafe { slice::from_raw_parts(input, length) }
            };
            evaluate_json(bytes)
        };
        CString::new(response.to_string()).map_or(ptr::null_mut(), CString::into_raw)
    });
    result.unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn unitsml_free_json_v1(reply: *mut c_char) {
    if !reply.is_null() {
        drop(unsafe { CString::from_raw(reply) });
    }
}
