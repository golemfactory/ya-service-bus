use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "flex")]
pub use flex::{DecodeError, EncodeError};
#[cfg(feature = "json")]
pub use json::{DecodeError, EncodeError};

lazy_static::lazy_static! {
    pub static ref CONFIG: Config = Config::default();
}

#[derive(Default)]
pub struct Config {
    compress: AtomicBool,
}

impl Config {
    pub fn set_compress(&self, val: bool) {
        self.compress.store(val, Ordering::SeqCst);
    }
}

pub fn to_vec<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
    #[cfg(feature = "flex")]
    use flex::to_vec;
    #[cfg(feature = "json")]
    use json::to_vec;

    to_vec(value).map(|vec| {
        if CONFIG.compress.load(Ordering::SeqCst) {
            miniz_oxide::deflate::compress_to_vec_zlib(vec.as_slice(), 6)
        } else {
            vec
        }
    })
}

pub fn from_slice<T: serde::de::DeserializeOwned>(slice: &[u8]) -> Result<T, DecodeError> {
    #[cfg(feature = "flex")]
    use flex::from_slice;
    #[cfg(feature = "json")]
    use json::from_slice;

    match miniz_oxide::inflate::decompress_to_vec_zlib(slice) {
        Ok(vec) => from_slice(vec.as_slice()),
        Err(_) => from_slice(slice),
    }
}

#[allow(dead_code)]
#[cfg(feature = "flex")]
mod flex {
    use flexbuffers::{DeserializationError, SerializationError};

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct DecodeError(DeserializationError);

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct EncodeError(SerializationError);

    #[inline]
    pub fn to_vec<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
        flexbuffers::to_vec(value).map_err(EncodeError)
    }

    #[inline]
    pub fn from_slice<T: serde::de::DeserializeOwned>(slice: &[u8]) -> Result<T, DecodeError> {
        flexbuffers::from_slice(slice).map_err(DecodeError)
    }
}

#[allow(dead_code)]
#[cfg(feature = "json")]
mod json {
    use serde_json::Error;

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct DecodeError(Error);

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct EncodeError(Error);

    #[inline]
    pub fn to_vec<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
        serde_json::to_vec(value).map_err(EncodeError)
    }

    #[inline]
    pub fn from_slice<T: serde::de::DeserializeOwned>(slice: &[u8]) -> Result<T, DecodeError> {
        serde_json::from_slice(slice).map_err(DecodeError)
    }
}
