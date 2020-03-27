use std::{convert::TryFrom, mem::size_of, net::SocketAddr};
use url::{ParseError, Url};

use crate::codec::ProtocolError;

pub use gsb_api::*;

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
    Ping = 12,
    Pong = 13,
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

pub const GSB_URL_ENV_VAR: &str = "GSB_URL";
pub const DEFAULT_GSB_URL: &str = "tcp://127.0.0.1:7464";

pub fn gsb_addr(gsb_url: Option<Url>) -> SocketAddr {
    let gsb_url = gsb_url.unwrap_or_else(|| {
        let default_url = std::env::var(GSB_URL_ENV_VAR).unwrap_or(DEFAULT_GSB_URL.into());
        match Url::parse(&default_url) {
            Err(ParseError::RelativeUrlWithoutBase) => {
                Url::parse(&format!("tcp://{}", default_url))
            }
            x => x,
        }
        .expect("provide GSB URL in format tcp://<ip:port>")
    });

    if gsb_url.scheme() != "tcp" {
        panic!("unimplemented protocol for GSB URL: {}", gsb_url.scheme());
    }
    let ip_addr = gsb_url
        .host_str()
        .expect("need IP address for GSB URL")
        .parse()
        .expect("only IP address supported for GSB URL");

    SocketAddr::new(
        ip_addr,
        gsb_url
            .port()
            .unwrap_or_else(|| Url::parse(DEFAULT_GSB_URL).unwrap().port().unwrap()),
    )
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;

    #[test]
    #[serial_test::serial]
    pub fn check_default_gsb_url() {
        std::env::remove_var(GSB_URL_ENV_VAR);
        let addr = gsb_addr(None);
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 7464)
    }

    #[test]
    #[serial_test::serial]
    pub fn check_env_var() {
        std::env::set_var(GSB_URL_ENV_VAR, "tcp://10.9.8.7:2345");
        let addr = gsb_addr(None);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7)));
        assert_eq!(addr.port(), 2345);
    }

    #[test]
    #[serial_test::serial]
    pub fn check_no_tcp_protocol_gsb_url() {
        std::env::set_var(GSB_URL_ENV_VAR, "10.9.8.7:1234");
        let addr = gsb_addr(None);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7)));
        assert_eq!(addr.port(), 1234)
    }

    #[test]
    #[serial_test::serial]
    pub fn check_ip_only_gsb_url() {
        std::env::set_var(GSB_URL_ENV_VAR, "10.9.8.7");
        let addr = gsb_addr(None);
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7)));
        assert_eq!(addr.port(), 7464)
    }

    #[test]
    #[serial_test::serial]
    #[should_panic(expected = "unimplemented protocol for GSB URL: http")]
    pub fn panic_env_var_http() {
        std::env::set_var(GSB_URL_ENV_VAR, "http://10.9.8.7:2345");
        gsb_addr(None);
    }

    #[test]
    pub fn check_ip_port_gsb_url() {
        let addr = gsb_addr(Some("tcp://10.9.8.7:2345".parse().unwrap()));
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7)));
        assert_eq!(addr.port(), 2345)
    }

    #[test]
    pub fn check_ip_gsb_url() {
        let addr = gsb_addr(Some("tcp://10.9.8.7".parse().unwrap()));
        assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7)));
        assert_eq!(addr.port(), 7464)
    }

    #[test]
    #[should_panic(expected = "unimplemented protocol for GSB URL: http")]
    pub fn panic_http_gsb_url() {
        gsb_addr(Some("http://10.9.8.7".parse().unwrap()));
    }

    #[test]
    #[should_panic(expected = "only IP address supported for GSB URL: AddrParseError(())")]
    pub fn panic_domain_gsb_url() {
        gsb_addr(Some("tcp://zima".parse().unwrap()));
    }

    #[test]
    #[should_panic(expected = "need IP address for GSB URL")]
    pub fn panic_no_host_gsb_url() {
        gsb_addr(Some("tcp:".parse().unwrap()));
    }
}
