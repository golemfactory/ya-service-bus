pub use gsb_api::*;
use std::convert::TryFrom;
use std::fmt::{Debug, Display, Formatter};
use std::path::PathBuf;
use url::{ParseError, Url};

mod gsb_api {
    include!(concat!(env!("OUT_DIR"), "/gsb_api.rs"));
}

#[cfg(feature = "with-codec")]
pub mod codec;

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
#[cfg(unix)]
pub const DEFAULT_GSB_URL: &str = "unix:/tmp/yagna.sock";
#[cfg(not(unix))]
pub const DEFAULT_GSB_URL: &str = "tcp://127.0.0.1:7464";
pub const DEFAULT_GSB_PORT: u16 = 7464;

#[derive(Clone, Debug)]
pub enum GsbAddr {
    Tcp(String),
    Unix(PathBuf),
}

impl GsbAddr {
    pub fn from_url(gsb_url: Option<Url>) -> Self {
        let gsb_url = gsb_url.unwrap_or_else(|| {
            let default_url =
                std::env::var(GSB_URL_ENV_VAR).unwrap_or_else(|_| DEFAULT_GSB_URL.into());
            match Url::parse(&default_url) {
                Err(ParseError::RelativeUrlWithoutBase) => {
                    Url::parse(&format!("tcp://{}", default_url))
                }
                x => x,
            }
            .expect("provide GSB URL in format tcp://<ip:port> or unix:<path>")
        });

        match gsb_url.scheme() {
            "tcp" => Self::Tcp(parse_tcp_url(gsb_url)),
            "unix" => Self::Unix(parse_unix_url(gsb_url)),
            _ => panic!("unimplemented protocol for GSB URL: {}", gsb_url.scheme()),
        }
    }
}

impl Default for GsbAddr {
    fn default() -> Self {
        GsbAddr::from_url(None)
    }
}

impl Display for GsbAddr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GsbAddr::Tcp(addr) => std::fmt::Display::fmt(addr, f),
            GsbAddr::Unix(path) => std::fmt::Display::fmt(&path.to_string_lossy(), f),
        }
    }
}

fn parse_tcp_url(url: Url) -> String {
    let host = url.host_str().expect("need host for GSB URL");
    let port = url.port().unwrap_or(DEFAULT_GSB_PORT);

    format!("{}:{}", host, port)
}

#[cfg(unix)]
fn parse_unix_url(url: Url) -> PathBuf {
    url.to_file_path().expect("invalid socket path in GSB URL")
}

#[cfg(not(unix))]
fn parse_unix_url(_url: Url) -> PathBuf {
    panic!("Unix sockets not supported on this OS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    pub fn check_default_gsb_url() {
        std::env::remove_var(GSB_URL_ENV_VAR);
        let addr = GsbAddr::from_url(None);
        let path = match addr {
            GsbAddr::Unix(path) => path,
            _ => panic!("Not a UNIX addr"),
        };
        assert_eq!(path, PathBuf::from("/tmp/yagna.sock"))
    }

    #[cfg(not(unix))]
    #[test]
    #[serial_test::serial]
    pub fn check_default_gsb_url() {
        std::env::remove_var(GSB_URL_ENV_VAR);
        let addr = GsbAddr::from_url(None);
        let addr = match addr {
            GsbAddr::Tcp(addr) => addr,
            _ => panic!("Not a TCP addr"),
        };
        assert_eq!(addr, "127.0.0.1:7464")
    }

    #[test]
    #[serial_test::serial]
    pub fn check_env_var() {
        std::env::set_var(GSB_URL_ENV_VAR, "tcp://10.9.8.7:2345");
        let addr = GsbAddr::from_url(None);
        let addr = match addr {
            GsbAddr::Tcp(addr) => addr,
            _ => panic!("Not a TCP addr"),
        };
        assert_eq!(addr, "10.9.8.7:2345");
    }

    #[test]
    #[serial_test::serial]
    pub fn check_no_tcp_protocol_gsb_url() {
        std::env::set_var(GSB_URL_ENV_VAR, "10.9.8.7:1234");
        let addr = GsbAddr::from_url(None);
        let addr = match addr {
            GsbAddr::Tcp(addr) => addr,
            _ => panic!("Not a TCP addr"),
        };
        assert_eq!(addr, "10.9.8.7:1234");
    }

    #[test]
    #[serial_test::serial]
    #[should_panic(expected = "unimplemented protocol for GSB URL: http")]
    pub fn panic_env_var_http() {
        std::env::set_var(GSB_URL_ENV_VAR, "http://10.9.8.7:2345");
        GsbAddr::from_url(None);
    }

    #[test]
    pub fn check_ip_port_gsb_url() {
        let addr = GsbAddr::from_url(Some("tcp://10.9.8.7:2345".parse().unwrap()));
        let addr = match addr {
            GsbAddr::Tcp(addr) => addr,
            _ => panic!("Not a TCP addr"),
        };
        assert_eq!(addr, "10.9.8.7:2345");
    }

    #[cfg(unix)]
    #[test]
    pub fn check_unix_gsb_url() {
        let addr = GsbAddr::from_url(Some("unix:/tmp/śmigły żółw/socket".parse().unwrap()));
        let path = match addr {
            GsbAddr::Unix(path) => path,
            _ => panic!("Not a UNIX addr"),
        };
        assert_eq!(path, PathBuf::from("/tmp/śmigły żółw/socket"))
    }

    #[cfg(not(unix))]
    #[test]
    #[should_panic(expected = "Unix sockets not supported on this OS")]
    pub fn check_unix_gsb_url() {
        GsbAddr::from_url(Some("unix:/tmp/socket".parse().unwrap()));
    }

    #[test]
    #[should_panic(expected = "unimplemented protocol for GSB URL: http")]
    pub fn panic_http_gsb_url() {
        GsbAddr::from_url(Some("http://10.9.8.7".parse().unwrap()));
    }

    #[test]
    #[should_panic(expected = "need host for GSB URL")]
    pub fn panic_no_host_gsb_url() {
        GsbAddr::from_url(Some("tcp:".parse().unwrap()));
    }
}
