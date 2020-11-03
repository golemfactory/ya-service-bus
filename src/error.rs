use super::serialization::{DecodeError, EncodeError};
use actix::MailboxError;
use futures::channel::oneshot;
use std::io;
use std::net::SocketAddr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bus connection to {0} fail: {1}")]
    BusConnectionFail(SocketAddr, io::Error),
    #[error("Called service `{0}` is unavailable")]
    Closed(String),
    #[error("Service receiver was cancelled")]
    Cancelled,
    #[error("No such endpoint `{0}`")]
    NoEndpoint(String),
    #[error("bad content {0}")]
    BadContent(#[from] DecodeError),
    #[error("{0}")]
    EncodingProblem(String),
    #[error("Timeout calling `{0}` service")]
    Timeout(String),
    #[error("bad request: {0}")]
    GsbBadRequest(String),
    #[error("already registered: {0}")]
    GsbAlreadyRegistered(String),
    #[error("{0}")]
    GsbFailure(String),
    #[error("Remote service at `{0}` error: {1}")]
    RemoteError(String, String),
}

impl From<MailboxError> for Error {
    fn from(e: MailboxError) -> Self {
        match e {
            MailboxError::Closed => Error::Closed("unknown".into()),
            MailboxError::Timeout => Error::Timeout("unknown".into()),
        }
    }
}

impl Error {
    pub(crate) fn from_addr(addr: String, e: MailboxError) -> Self {
        match e {
            MailboxError::Closed => Error::Closed(addr),
            MailboxError::Timeout => Error::Timeout(addr),
        }
    }
}

impl From<oneshot::Canceled> for Error {
    fn from(_: oneshot::Canceled) -> Self {
        Error::Cancelled
    }
}

impl From<EncodeError> for Error {
    fn from(e: EncodeError) -> Self {
        Error::EncodingProblem(format!("{}", e))
    }
}
