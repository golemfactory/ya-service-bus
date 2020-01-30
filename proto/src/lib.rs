use std::convert::TryFrom;
use std::mem::size_of;

use crate::codec::ProtocolError;
pub use gsb_api::*;
use std::borrow::Cow;
use std::net::SocketAddr;

mod gsb_api {
    include!(concat!(env!("OUT_DIR"), "/gsb_api.rs"));
}

#[cfg(feature = "with-codec")]
pub mod codec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum MessageType {
    RegisterRequest = 0,
    RegisterReply = 1,
    UnregisterRequest = 2,
    UnregisterReply = 3,
    CallRequest = 4,
    CallReply = 5,
    SubscribeRequest = 6,
    SubscribeReply = 7,
    UnsubscribeRequest = 8,
    UnsubscribeReply = 9,
    BroadcastRequest = 10,
    BroadcastReply = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageHeader {
    pub msg_type: i32,
    pub msg_length: u32,
}

impl MessageHeader {
    pub fn encode(&self, buf: &mut tokio_bytes::BytesMut) {
        buf.extend_from_slice(&self.msg_type.to_be_bytes());
        buf.extend_from_slice(&self.msg_length.to_be_bytes());
    }

    pub fn decode(mut src: tokio_bytes::BytesMut) -> Result<Self, ProtocolError> {
        if src.len() < size_of::<MessageHeader>() {
            return Err(ProtocolError::HeaderNotEnoughBytes);
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

#[derive(thiserror::Error, Debug)]
#[error("invalid value: {0}")]
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

pub const DEFAULT_GSB_URL :&str = "tcp://127.0.0.1:7464";

pub fn gsb_url() -> Cow<'static, str> {
    if let Some(gsb_url) = std::env::var("GSB_URL").ok() {
        Cow::Owned(gsb_url)
    }
    else {
        Cow::Borrowed(DEFAULT_GSB_URL)
    }
}

pub fn gsb_addr() -> SocketAddr {
    let gsb_url = gsb_url();
    let url = url::Url::parse(&gsb_url).unwrap();
    if url.scheme() != "tcp" {
        panic!("unimplemented protocol: {}", url.scheme());
    }

    SocketAddr::new(url.host_str().unwrap().parse().unwrap(), url.port().unwrap_or(7464))
}