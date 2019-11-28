#![allow(unused)]

use actix::MailboxError;
use failure::Fail;
use std::io;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "bus connection fail: {}", _0)]
    BusConnectionFail(io::Error),
    #[fail(display = "Mailbox has closed")]
    Closed,
    #[fail(display = "Message delivery timed out")]
    Timeout,
}

impl From<MailboxError> for Error {
    fn from(e: MailboxError) -> Self {
        match e {
            MailboxError::Closed => Error::Closed,
            MailboxError::Timeout => Error::Timeout,
        }
    }
}
