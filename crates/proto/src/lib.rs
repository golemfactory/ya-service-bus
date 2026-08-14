pub use gsb_api::*;
use std::convert::TryFrom;
use std::fmt::{Debug, Display, Formatter};
use std::path::{Path, PathBuf};
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
#[deprecated(note = "use default_gsb_url() to resolve the platform default")]
pub const DEFAULT_GSB_URL: &str = "unix:/tmp/yagna.sock";
#[cfg(not(unix))]
pub const DEFAULT_GSB_URL: &str = "tcp://127.0.0.1:7464";
pub const DEFAULT_GSB_PORT: u16 = 7464;

#[cfg(unix)]
const APP_DIR: &str = "yagna";
#[cfg(unix)]
const RUN_DIR: &str = "run";
#[cfg(unix)]
const SOCKET_NAME: &str = "gsb.sock";

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultUnixGsbEndpoint {
    pub socket_path: PathBuf,
    pub trusted_root: PathBuf,
    pub private_dirs: Vec<PathBuf>,
}

#[cfg(unix)]
pub fn linux_default_gsb_endpoint(
    data_dir: &Path,
    is_default_data_dir: bool,
    runtime_dir: Option<&Path>,
) -> DefaultUnixGsbEndpoint {
    if let Some(runtime_dir) = runtime_dir.filter(|path| path.is_absolute()) {
        let app_dir = runtime_dir.join(APP_DIR);
        if is_default_data_dir {
            return DefaultUnixGsbEndpoint {
                socket_path: app_dir.join(SOCKET_NAME),
                trusted_root: runtime_dir.to_path_buf(),
                private_dirs: vec![app_dir],
            };
        }

        let instance_dir = app_dir.join(data_dir_namespace(data_dir));
        return DefaultUnixGsbEndpoint {
            socket_path: instance_dir.join(SOCKET_NAME),
            trusted_root: runtime_dir.to_path_buf(),
            private_dirs: vec![app_dir, instance_dir],
        };
    }

    let private_dir = data_dir.join(RUN_DIR);
    DefaultUnixGsbEndpoint {
        socket_path: private_dir.join(SOCKET_NAME),
        trusted_root: data_dir.to_path_buf(),
        private_dirs: vec![private_dir],
    }
}

#[cfg(unix)]
pub fn macos_default_gsb_endpoint(data_dir: &Path, temp_dir: &Path) -> DefaultUnixGsbEndpoint {
    let app_dir = temp_dir.join(APP_DIR);
    let instance_dir = app_dir.join(data_dir_namespace(data_dir));
    DefaultUnixGsbEndpoint {
        socket_path: instance_dir.join(SOCKET_NAME),
        trusted_root: temp_dir.to_path_buf(),
        private_dirs: vec![app_dir, instance_dir],
    }
}

#[cfg(unix)]
pub fn default_unix_gsb_endpoint(
    data_dir: &Path,
    is_default_data_dir: bool,
) -> DefaultUnixGsbEndpoint {
    #[cfg(target_os = "linux")]
    {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        linux_default_gsb_endpoint(data_dir, is_default_data_dir, runtime_dir.as_deref())
    }

    #[cfg(target_os = "macos")]
    {
        macos_default_gsb_endpoint(data_dir, &std::env::temp_dir())
    }

    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    {
        linux_default_gsb_endpoint(data_dir, is_default_data_dir, None)
    }
}

#[cfg(unix)]
fn default_yagna_data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "GolemFactory", APP_DIR)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("GolemFactory").join(APP_DIR))
}

#[cfg(unix)]
fn data_dir_namespace(data_dir: &Path) -> String {
    // Stable filesystem namespacing only; this is not a security primitive.
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let hash = data_dir
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(OFFSET, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        });
    format!("{hash:016x}")
}

pub fn default_gsb_url() -> Url {
    #[cfg(unix)]
    {
        let endpoint = default_unix_gsb_endpoint(&default_yagna_data_dir(), true);
        let file_url = Url::from_file_path(&endpoint.socket_path).unwrap_or_else(|_| {
            panic!(
                "cannot encode default GSB socket path {} as a URL; set {GSB_URL_ENV_VAR}",
                endpoint.socket_path.display()
            )
        });
        Url::parse(&format!("unix:{}", file_url.path()))
            .expect("cannot encode default GSB socket URL")
    }

    #[cfg(not(unix))]
    {
        Url::parse(DEFAULT_GSB_URL).expect("invalid platform DEFAULT_GSB_URL")
    }
}

#[derive(Clone, Debug)]
pub enum GsbAddr {
    Tcp(String),
    Unix(PathBuf),
}

impl GsbAddr {
    pub fn from_url(gsb_url: Option<Url>) -> Self {
        let gsb_url = gsb_url.unwrap_or_else(|| {
            std::env::var(GSB_URL_ENV_VAR).map_or_else(
                |_| default_gsb_url(),
                |url| {
                    match Url::parse(&url) {
                        Err(ParseError::RelativeUrlWithoutBase) => {
                            Url::parse(&format!("tcp://{}", url))
                        }
                        parsed => parsed,
                    }
                    .expect("provide GSB URL in format tcp://<ip:port> or unix:<path>")
                },
            )
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
        assert_eq!(
            path,
            default_unix_gsb_endpoint(&default_yagna_data_dir(), true).socket_path
        )
    }

    #[cfg(unix)]
    #[test]
    fn linux_defaults_match_yagna_layout() {
        let runtime_dir = Path::new("/run/user/1000");
        let data_dir = Path::new("/home/user/.local/share/yagna");

        let runtime = linux_default_gsb_endpoint(data_dir, true, Some(runtime_dir));
        assert_eq!(
            runtime.socket_path,
            PathBuf::from("/run/user/1000/yagna/gsb.sock")
        );

        let fallback = linux_default_gsb_endpoint(data_dir, true, None);
        assert_eq!(
            fallback.socket_path,
            PathBuf::from("/home/user/.local/share/yagna/run/gsb.sock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn custom_data_dirs_get_distinct_runtime_endpoints() {
        let runtime_dir = Path::new("/run/user/1000");
        let first = linux_default_gsb_endpoint(Path::new("/data/one"), false, Some(runtime_dir));
        let second = linux_default_gsb_endpoint(Path::new("/data/two"), false, Some(runtime_dir));

        assert_ne!(first.socket_path, second.socket_path);
        assert!(first.socket_path.starts_with(runtime_dir.join(APP_DIR)));
        assert!(second.socket_path.starts_with(runtime_dir.join(APP_DIR)));
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
