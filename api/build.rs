use std::env;

fn main() {
    env::set_var("OUT_DIR", "src");
    prost_build::compile_protos(
        &["protos/gsb_api.proto"],
        &["protos/"]
    ).unwrap();
}
