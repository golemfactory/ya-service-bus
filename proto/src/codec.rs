use std::convert::TryInto;
use std::mem::size_of;

use crate::gsb_api::*;
use bytes::BytesMut;
use prost::Message;
use tokio_codec::{Decoder, Encoder};

use crate::{MessageHeader, MessageType};

const MSG_HEADER_LENGTH: usize = size_of::<MessageHeader>();

pub type ProtocolError = failure::Error;

trait Encodable {
    // This trait exists because prost::Message has template methods

    fn encode_(&self, buf: &mut BytesMut) -> failure::Fallible<()>;
    fn encoded_len_(&self) -> usize;
}

impl<T: Message> Encodable for T {
    fn encode_(&self, buf: &mut BytesMut) -> failure::Fallible<()> {
        Ok(self.encode(buf)?)
    }

    fn encoded_len_(&self) -> usize {
        self.encoded_len()
    }
}

#[derive(Debug)]
pub enum GsbMessage {
    RegisterRequest(RegisterRequest),
    RegisterReply(RegisterReply),
    UnregisterRequest(UnregisterRequest),
    UnregisterReply(UnregisterReply),
    CallRequest(CallRequest),
    CallReply(CallReply),
}

impl GsbMessage {
    fn unpack(self) -> (MessageType, Box<dyn Encodable>) {
        match self {
            GsbMessage::RegisterRequest(msg) => (MessageType::RegisterRequest, Box::new(msg)),
            GsbMessage::RegisterReply(msg) => (MessageType::RegisterReply, Box::new(msg)),
            GsbMessage::UnregisterRequest(msg) => (MessageType::UnregisterRequest, Box::new(msg)),
            GsbMessage::UnregisterReply(msg) => (MessageType::UnregisterReply, Box::new(msg)),
            GsbMessage::CallRequest(msg) => (MessageType::CallRequest, Box::new(msg)),
            GsbMessage::CallReply(msg) => (MessageType::CallReply, Box::new(msg)),
        }
    }
}

impl Into<GsbMessage> for RegisterRequest {
    fn into(self) -> GsbMessage {
        GsbMessage::RegisterRequest(self)
    }
}

impl Into<GsbMessage> for RegisterReply {
    fn into(self) -> GsbMessage {
        GsbMessage::RegisterReply(self)
    }
}

impl Into<GsbMessage> for UnregisterRequest {
    fn into(self) -> GsbMessage {
        GsbMessage::UnregisterRequest(self)
    }
}

impl Into<GsbMessage> for UnregisterReply {
    fn into(self) -> GsbMessage {
        GsbMessage::UnregisterReply(self)
    }
}

impl Into<GsbMessage> for CallRequest {
    fn into(self) -> GsbMessage {
        GsbMessage::CallRequest(self)
    }
}

impl Into<GsbMessage> for CallReply {
    fn into(self) -> GsbMessage {
        GsbMessage::CallReply(self)
    }
}

fn decode_header(src: &mut BytesMut) -> failure::Fallible<Option<MessageHeader>> {
    if src.len() < MSG_HEADER_LENGTH {
        Ok(None)
    } else {
        let buf = src.split_to(MSG_HEADER_LENGTH);
        Ok(Some(MessageHeader::decode(buf)?))
    }
}

fn decode_message(
    src: &mut BytesMut,
    header: &MessageHeader,
) -> failure::Fallible<Option<GsbMessage>> {
    let msg_length = header.msg_length.try_into()?;
    if src.len() < msg_length {
        Ok(None)
    } else {
        let buf = src.split_to(msg_length);
        let msg_type = MessageType::from_i32(header.msg_type);
        let msg: GsbMessage = match msg_type {
            Some(MessageType::RegisterRequest) => RegisterRequest::decode(buf)?.into(),
            Some(MessageType::RegisterReply) => RegisterReply::decode(buf)?.into(),
            Some(MessageType::UnregisterRequest) => UnregisterRequest::decode(buf)?.into(),
            Some(MessageType::UnregisterReply) => UnregisterReply::decode(buf)?.into(),
            Some(MessageType::CallRequest) => CallRequest::decode(buf)?.into(),
            Some(MessageType::CallReply) => CallReply::decode(buf)?.into(),
            None => {
                return Err(failure::err_msg(format!(
                    "Unrecognized message type: {}",
                    header.msg_type
                )))
            }
        };
        Ok(Some(msg))
    }
}

fn encode_message(dst: &mut BytesMut, msg: GsbMessage) -> failure::Fallible<()> {
    let (msg_type, msg) = msg.unpack();
    encode_message_unpacked(dst, msg_type, msg.as_ref())?;
    Ok(())
}

fn encode_message_unpacked(
    dst: &mut BytesMut,
    msg_type: MessageType,
    msg: &dyn Encodable,
) -> failure::Fallible<()> {
    let msg_type = msg_type as i32;
    let msg_length = msg.encoded_len_() as u32;
    let header = MessageHeader {
        msg_type,
        msg_length,
    };
    header.encode(dst);
    msg.encode_(dst)?;
    Ok(())
}

pub struct GsbMessageDecoder {
    msg_header: Option<MessageHeader>,
}

impl GsbMessageDecoder {
    pub fn new() -> Self {
        GsbMessageDecoder { msg_header: None }
    }
}

impl Decoder for GsbMessageDecoder {
    type Item = GsbMessage;
    type Error = failure::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.msg_header == None {
            self.msg_header = decode_header(src)?;
        }
        match &self.msg_header {
            None => Ok(None),
            Some(header) => match decode_message(src, &header)? {
                None => {
                    src.reserve(header.msg_length as usize);
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

pub struct GsbMessageEncoder;

impl Encoder for GsbMessageEncoder {
    type Item = GsbMessage;
    type Error = failure::Error;

    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        encode_message(dst, item)
    }
}
