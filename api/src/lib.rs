mod gsb_api {
    include!(concat!(env!("OUT_DIR"), "/gsb_api.rs"));
}

pub use gsb_api::*;
