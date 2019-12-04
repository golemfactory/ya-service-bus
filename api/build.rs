use std::env;

fn main() {
    println!(
        "cargo:warning=Generating code into {}",
        env::var("OUT_DIR").unwrap()
    );
    prost_build::compile_protos(&["protos/gsb_api.proto"], &["protos/"]).unwrap();
}
