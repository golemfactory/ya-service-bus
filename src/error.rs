use super::serialization::{DecodeError, EncodeError};
use actix::MailboxError;
use std::io;

#[derive(Clone, Debug, thiserror::Error)]
#[error("Timeout connecting GSB at `{0}`")]
pub struct ConnectionTimeout(pub ya_sb_proto::GsbAddr);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Connecting GSB at `{0}` failure: {1}")]
    ConnectionFail(ya_sb_proto::GsbAddr, io::Error),
    #[error(transparent)]
    ConnectionTimeout(#[from] ConnectionTimeout),
    #[error("Called service `{0}` is unavailable")]
    Closed(String),
    #[error("Service receiver was cancelled")]
    Cancelled,
    #[error("No such endpoint `{0}`")]
    NoEndpoint(String),
    #[error("Bad content: {0}")]
    BadContent(#[from] DecodeError),
    #[error("Encoding problem: {0}")]
    EncodingProblem(String),
    #[error("Timeout calling `{0}` service")]
    Timeout(String),
    #[error("Bad request: {0}")]
    GsbBadRequest(String),
    #[error("Already registered: `{0}`")]
    GsbAlreadyRegistered(String),
    #[error("GSB failure: {0}")]
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

impl From<EncodeError> for Error {
    fn from(e: EncodeError) -> Self {
        Error::EncodingProblem(format!("{}", e))
    }
}
