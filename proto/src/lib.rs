use failure::Fail;

mod gsb_api {
    include!(concat!(env!("OUT_DIR"), "/gsb_api.rs"));
}

use bytes::BytesMut;
use failure;
use std::mem::size_of;

#[cfg(feature = "with-codec")]
pub mod codec;
#[cfg(feature = "with-codec")]
pub mod decoder;

pub use gsb_api::*;
use std::convert::TryFrom;
use std::fs::read;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum MessageType {
    RegisterRequest = 0,
    RegisterReply = 1,
    UnregisterRequest = 2,
    UnregisterReply = 3,
    CallRequest = 4,
    CallReply = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageHeader {
    pub msg_type: i32,
    pub msg_length: u32,
}

impl MessageHeader {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.extend_from_slice(&self.msg_type.to_be_bytes());
        buf.extend_from_slice(&self.msg_length.to_be_bytes());
    }

    pub fn decode(mut src: BytesMut) -> failure::Fallible<Self> {
        if src.len() < size_of::<MessageHeader>() {
            return Err(failure::err_msg(
                "Cannot decode message header: not enough bytes",
            ));
        }

        let mut msg_type: [u8; 4] = [0; 4];
        let mut msg_length: [u8; 4] = [0; 4];
        msg_type.copy_from_slice(src.split_to(size_of::<i32>()).as_ref());
        msg_length.copy_from_slice(src.split_to(size_of::<u32>()).as_ref());

        Ok(MessageHeader {
            msg_type: i32::from_be_bytes(msg_type),
            msg_length: u32::from_be_bytes(msg_length),
        })
    }
}

#[derive(Fail, Debug)]
#[fail(display = "invalid value: {}", _0)]
pub struct EnumError(pub i32);

impl TryFrom<i32> for CallReplyCode {
    type Error = EnumError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => CallReplyCode::CallReplyOk,
            400 => CallReplyCode::CallReplyBadRequest,
            500 => CallReplyCode::ServiceFailure,
            _ => return Err(EnumError(value)),
        })
    }
}

impl TryFrom<i32> for CallReplyType {
    type Error = EnumError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => CallReplyType::Full,
            1 => CallReplyType::Partial,
            _ => return Err(EnumError(value)),
        })
    }
}
