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
    pub struct DecodeError(#[from] Error);

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

#[cfg(feature = "flex")]
pub use flex::*;

#[cfg(feature = "json")]
pub use json::*;
