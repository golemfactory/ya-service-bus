#[cfg(feature = "msgpack")]
pub use rmp_serde::{
    decode::Error as DecodeError, encode::Error as EncodeError, from_read, to_vec_named as to_vec,
};

#[cfg(feature = "json")]
mod json {
    use serde_json::Error;
    use std::fmt;

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
    pub fn from_read<T: serde::de::DeserializeOwned, R: std::io::Read>(
        read: R,
    ) -> Result<T, DecodeError> {
        serde_json::from_reader(read).map_err(DecodeError)
    }
}

#[cfg(feature = "json")]
pub use json::*;
