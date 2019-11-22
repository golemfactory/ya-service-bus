use std::convert::TryInto;

use bytes::BytesMut;
use prost::Message;
use tokio::codec::Decoder;

use ya_sb_api::*;

lazy_static! {
    static ref MSG_HEADER_LENGTH: usize = {
        let header = MessageHeader{msg_type: 0, msg_length: 0};
        header.encoded_len()
    };
}

pub enum IncomingGsbMessage {
    RegisterRequest(RegisterRequest),
    UnregisterRequest(UnregisterRequest),
    ServiceCallRequest(ServiceCallRequest),
    CallReply(CallReply)
}

fn parse_header(src: &mut BytesMut) -> failure::Fallible<Option<MessageHeader>> {
    if src.len() < *MSG_HEADER_LENGTH {
        Ok(None)
    } else {
        let buf = src.split_to(*MSG_HEADER_LENGTH + 1);
        Ok(Some(MessageHeader::decode(buf)?))
    }
}

fn parse_message(src: &mut BytesMut, header: &MessageHeader) -> failure::Fallible<Option<IncomingGsbMessage>> {
    let msg_length = header.msg_length.try_into()?;
    if src.len() < msg_length {
        Ok(None)
    } else {
        let buf = src.split_to(msg_length + 1);
        let msg_type = MessageType::from_i32(header.msg_type);
        let msg = match msg_type {
            Some(MessageType::RegisterRequest) => IncomingGsbMessage::RegisterRequest(RegisterRequest::decode(buf)?),
            Some(MessageType::UnregisterRequest) => IncomingGsbMessage::UnregisterRequest(UnregisterRequest::decode(buf)?),
            Some(MessageType::ServiceCallRequest) => IncomingGsbMessage::ServiceCallRequest(ServiceCallRequest::decode(buf)?),
            Some(MessageType::CallReply) => IncomingGsbMessage::CallReply(CallReply::decode(buf)?),
            _ => return Err(failure::err_msg("Unsupported message type"))
        };
        Ok(Some(msg))
    }
}

pub struct GsbMessageDecoder {
    msg_header: Option<MessageHeader>
}

impl GsbMessageDecoder {
    pub fn new() -> Self {
        GsbMessageDecoder { msg_header: None }
    }
}

impl Decoder for GsbMessageDecoder {
    type Item = IncomingGsbMessage;
    type Error = failure::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.msg_header == None {
            self.msg_header = parse_header(src)?;
        }
        match &self.msg_header {
            None => Ok(None),
            Some(header) => match parse_message(src, &header)? {
                None => Ok(None),
                Some(msg) => {
                    self.msg_header = None;
                    Ok(Some(msg))
                }
            }
        }
    }
}
