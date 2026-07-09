use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR should be set"));
    File::create(out.join("memory.x"))
        .expect("memory.x should be creatable")
        .write_all(include_bytes!("memory.x"))
        .expect("memory.x should be writable");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
