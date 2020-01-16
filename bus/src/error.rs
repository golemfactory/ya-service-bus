use actix::MailboxError;
use failure::Fail;
use futures::channel::oneshot;
use std::io;
use std::net::SocketAddr;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "bus connection to {} fail: {}", _0, _1)]
    BusConnectionFail(SocketAddr, io::Error),
    #[fail(display = "Mailbox has closed")]
    Closed,
    #[fail(display = "has closed")]
    NoEndpoint,
    #[fail(display = "bad content {}", _0)]
    BadContent(#[cause] rmp_serde::decode::Error),
    #[fail(display = "{}", _0)]
    EncodingProblem(String),
    #[fail(display = "Message delivery timed out")]
    Timeout,
    #[fail(display = "bad request: {}", _0)]
    GsbBadRequest(String),
    #[fail(display = "already registered: {}", _0)]
    GsbAlreadyRegistered(String),
    #[fail(display = "{}", _0)]
    GsbFailure(String),
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
        Error::Closed
    }
}

impl From<rmp_serde::decode::Error> for Error {
    fn from(e: rmp_serde::decode::Error) -> Self {
        Error::BadContent(e)
    }
}

impl From<rmp_serde::encode::Error> for Error {
    fn from(e: rmp_serde::encode::Error) -> Self {
        Error::EncodingProblem(format!("{}", e))
    }
}
