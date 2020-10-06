use super::serialization::{DecodeError, EncodeError};
use actix::MailboxError;
use futures::channel::oneshot;
use std::io;
use std::net::SocketAddr;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bus connection to {0} fail: {1}")]
    BusConnectionFail(SocketAddr, io::Error),
    #[error("The called GSB service is unavailable")]
    Closed,
    #[error("GSB receiver is cancelled")]
    Cancelled,
    #[error("has closed")]
    NoEndpoint,
    #[error("bad content {0}")]
    BadContent(#[from] DecodeError),
    #[error("{0}")]
    EncodingProblem(String),
    #[error("Message delivery timed out")]
    Timeout,
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
            MailboxError::Closed => Error::Closed,
            MailboxError::Timeout => Error::Timeout,
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
