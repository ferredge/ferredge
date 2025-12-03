use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

pub fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new(msg).ok());
}

#[no_mangle]
pub extern "C" fn ferredge_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow().as_ref().map(|s| s.as_ptr()).unwrap_or(std::ptr::null())
    })
}