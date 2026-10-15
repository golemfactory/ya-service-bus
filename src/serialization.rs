use miniz_oxide::inflate::TINFLStatus;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "flex")]
pub use flex::{DecodeError, EncodeError};
#[cfg(feature = "json")]
pub use json::{DecodeError, EncodeError};

/// Upper bound for a zlib-inflated GSB body. Bodies are inflated in memory before
/// deserialization, so without a cap a small compressed payload could exhaust memory.
pub const MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;

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

    match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(slice, MAX_DECOMPRESSED_SIZE) {
        Ok(vec) => from_slice(vec.as_slice()),
        Err(TINFLStatus::HasMoreOutput) => Err(DecodeError::TooLarge(MAX_DECOMPRESSED_SIZE)),
        Err(_) => from_slice(slice),
    }
}

#[allow(dead_code)]
#[cfg(feature = "flex")]
mod flex {
    use flexbuffers::{DeserializationError, SerializationError};

    #[derive(Debug, thiserror::Error)]
    pub enum DecodeError {
        #[error("{0}")]
        Deserialize(DeserializationError),
        #[error("decompressed body exceeds {0} bytes")]
        TooLarge(usize),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct EncodeError(SerializationError);

    #[inline]
    pub fn to_vec<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
        flexbuffers::to_vec(value).map_err(EncodeError)
    }

    #[inline]
    pub fn from_slice<T: serde::de::DeserializeOwned>(slice: &[u8]) -> Result<T, DecodeError> {
        flexbuffers::from_slice(slice).map_err(DecodeError::Deserialize)
    }
}

#[allow(dead_code)]
#[cfg(feature = "json")]
mod json {
    use serde_json::Error;

    #[derive(Debug, thiserror::Error)]
    pub enum DecodeError {
        #[error("{0}")]
        Deserialize(Error),
        #[error("decompressed body exceeds {0} bytes")]
        TooLarge(usize),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    pub struct EncodeError(Error);

    #[inline]
    pub fn to_vec<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, EncodeError> {
        serde_json::to_vec(value).map_err(EncodeError)
    }

    #[inline]
    pub fn from_slice<T: serde::de::DeserializeOwned>(slice: &[u8]) -> Result<T, DecodeError> {
        serde_json::from_slice(slice).map_err(DecodeError::Deserialize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Msg {
        name: String,
        data: Vec<u8>,
    }

    fn msg() -> Msg {
        Msg {
            name: "hello".into(),
            data: vec![7; 4096],
        }
    }

    #[test]
    fn decodes_uncompressed_body() {
        #[cfg(feature = "flex")]
        let body = flexbuffers::to_vec(msg()).unwrap();
        #[cfg(feature = "json")]
        let body = serde_json::to_vec(&msg()).unwrap();

        assert_eq!(from_slice::<Msg>(&body).unwrap(), msg());
    }

    #[test]
    fn decodes_compressed_body() {
        #[cfg(feature = "flex")]
        let body = flexbuffers::to_vec(msg()).unwrap();
        #[cfg(feature = "json")]
        let body = serde_json::to_vec(&msg()).unwrap();
        let body = miniz_oxide::deflate::compress_to_vec_zlib(&body, 6);

        assert_eq!(from_slice::<Msg>(&body).unwrap(), msg());
    }

    /// Builds a valid zlib stream of `len` zero bytes (fixed Huffman block made of
    /// a single literal followed by 258-byte back-references), without allocating
    /// the decompressed output.
    fn zlib_bomb(len: usize) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let (mut acc, mut nbits) = (0u32, 0u32);
        let mut put = |out: &mut Vec<u8>, code: u32, n: u32, reverse: bool| {
            let code = if reverse {
                code.reverse_bits() >> (32 - n)
            } else {
                code
            };
            acc |= code << nbits;
            nbits += n;
            while nbits >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                nbits -= 8;
            }
        };
        put(&mut out, 0b011, 3, false); // BFINAL=1, BTYPE=01 (fixed Huffman)
        put(&mut out, 0x30, 8, true); // literal 0
        for _ in 0..(len - 1) / 258 {
            put(&mut out, 0xc5, 8, true); // length 258
            put(&mut out, 0, 5, true); // distance 1
        }
        put(&mut out, 0, 7, true); // end of block
        put(&mut out, 0, 7, false); // flush
        let total = 1 + (len - 1) / 258 * 258;
        let adler = (((total % 65521) as u32) << 16) | 1;
        out.extend_from_slice(&adler.to_be_bytes());
        out
    }

    #[test]
    fn zlib_bomb_is_valid() {
        let bomb = zlib_bomb(1 + 258 * 1000);
        let out = miniz_oxide::inflate::decompress_to_vec_zlib(&bomb).unwrap();
        assert_eq!(out, vec![0u8; 1 + 258 * 1000]);
    }

    #[test]
    fn rejects_zlib_bomb() {
        let bomb = zlib_bomb(200 * 1024 * 1024);
        assert!(bomb.len() < 2 * 1024 * 1024);

        let result = from_slice::<Msg>(&bomb);
        assert!(matches!(
            result,
            Err(DecodeError::TooLarge(MAX_DECOMPRESSED_SIZE))
        ));
    }
}
