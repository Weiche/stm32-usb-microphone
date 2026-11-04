use std::{env, fs, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        fs::copy("memory.x", out.join("memory.x")).unwrap();
        println!("cargo:rustc-link-search={}", out.display());
    }
    println!("cargo:rerun-if-changed=memory.x");
}
