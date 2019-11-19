use serde::{Serialize, de::DeserializeOwned};

pub struct BusPath(Vec<String>);

impl From<&[&str]> for BusPath {

    fn from(path: &[&str]) -> Self {
        BusPath(path.into_iter().map(|&s| s.into()).collect())
    }

}

pub trait RpcMessage : Clone + Serialize + DeserializeOwned + 'static + Sync + Send {
    const ID : &'static str;
}

