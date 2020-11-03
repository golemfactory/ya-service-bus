use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_SHOW_GENPATH");
    if env::var("BUILD_SHOW_GENPATH").is_ok() {
        println!(
            "cargo:warning=Generating code into {}",
            env::var("OUT_DIR").unwrap()
        );
    }
    prost_build::compile_protos(&["protos/gsb_api.proto"], &["protos/"]).unwrap();
}
