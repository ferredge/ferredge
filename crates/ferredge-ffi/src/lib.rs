//! C-compatible FFI bindings for ferredge.

mod client;
mod error;
mod types;

pub use client::*;
pub use error::*;
pub use types::*;

use std::ffi::CString;
use std::os::raw::c_char;

#[no_mangle]
pub unsafe extern "C" fn ferredge_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[no_mangle]
pub extern "C" fn ferredge_version() -> *const c_char {
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}