use prost::Message;
use std::convert::TryInto;

use crate::gsb_api::*;
use thiserror::Error;

use bytes::{Buf, BufMut};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Unrecognized message type")]
    UnrecognizedMessageType,
    #[error("Cannot decode message header: not enough bytes")]
    HeaderNotEnoughBytes,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("encode error: {0}")]
    Encode(#[from] prost::EncodeError),
    #[error("decode {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("channel Receiver error")]
    RecvError,
    #[error("packet too big")]
    MsgTooBig,
}

trait Encodable {
    // This trait exists because prost::Message has template methods

    fn encode_(&self, buf: &mut bytes::BytesMut) -> Result<(), ProtocolError>;
    fn encoded_len_(&self) -> usize;
}

impl<T: Message> Encodable for T {
    fn encode_(&self, mut buf: &mut bytes::BytesMut) -> Result<(), ProtocolError> {
        Ok(self.encode(&mut buf)?)
    }

    fn encoded_len_(&self) -> usize {
        self.encoded_len()
    }
}

pub type GsbMessage = packet::Packet;

impl GsbMessage {
    pub fn pong() -> GsbMessage {
        packet::Packet::Pong(Pong {})
    }
}

macro_rules! into_packet {
    ($($t:ident),*) => {
        $(
        #[allow(clippy::from_over_into)]
        impl Into<packet::Packet> for $t {
            fn into(self) -> packet::Packet {
                packet::Packet::$t(self)
            }
        }
        )*
    };
}

into_packet! {
    RegisterRequest,
    RegisterReply,
    UnregisterRequest,
    UnregisterReply,
    CallRequest,
    CallReply,
    SubscribeRequest,
    SubscribeReply,
    UnsubscribeRequest,
    UnsubscribeReply,
    BroadcastRequest,
    BroadcastReply,
    Ping,
    Pong
}

fn decode_header(src: &mut bytes::BytesMut) -> Result<Option<u32>, ProtocolError> {
    if src.len() < 4 {
        Ok(None)
    } else {
        let mut buf = src.split_to(4);
        Ok(Some(buf.get_u32()))
    }
}

fn decode_message(
    src: &mut bytes::BytesMut,
    msg_length: u32,
) -> Result<Option<GsbMessage>, ProtocolError> {
    let msg_length = msg_length
        .try_into()
        .map_err(|_| ProtocolError::MsgTooBig)?;
    if src.len() < msg_length {
        Ok(None)
    } else {
        let buf = src.split_to(msg_length);
        let packet = Packet::decode(buf.as_ref())?;
        match packet.packet {
            Some(msg) => Ok(Some(msg)),
            None => Err(ProtocolError::UnrecognizedMessageType),
        }
    }
}

fn encode_message(dst: &mut bytes::BytesMut, msg: GsbMessage) -> Result<(), ProtocolError> {
    let packet = Packet { packet: Some(msg) };
    let len = packet.encoded_len();
    dst.put_u32(len as u32);
    packet.encode(dst)?;
    Ok(())
}

#[derive(Default)]
pub struct GsbMessageDecoder {
    msg_header: Option<u32>,
}

impl GsbMessageDecoder {
    pub fn new() -> Self {
        GsbMessageDecoder { msg_header: None }
    }
}

impl Decoder for GsbMessageDecoder {
    type Item = GsbMessage;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.msg_header.is_none() {
            self.msg_header = decode_header(src)?;
        }
        match self.msg_header {
            None => Ok(None),
            Some(msg_length) => match decode_message(src, msg_length)? {
                None => {
                    src.reserve(msg_length as usize);
                    Ok(None)
                }
                Some(msg) => {
                    self.msg_header = None;
                    Ok(Some(msg))
                }
            },
        }
    }
}

#[derive(Default)]
pub struct GsbMessageEncoder;

impl Encoder<GsbMessage> for GsbMessageEncoder {
    type Error = ProtocolError;

    fn encode(&mut self, item: GsbMessage, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        encode_message(dst, item)
    }
}

#[derive(Default)]
pub struct GsbMessageCodec {
    encoder: GsbMessageEncoder,
    decoder: GsbMessageDecoder,
}

impl Encoder<GsbMessage> for GsbMessageCodec {
    type Error = ProtocolError;

    fn encode(&mut self, item: GsbMessage, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        self.encoder.encode(item, dst)
    }
}

impl Decoder for GsbMessageCodec {
    type Item = GsbMessage;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.decoder.decode(src)
    }
}
