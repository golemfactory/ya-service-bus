use ya_sb_proto::{codec, decoder};

use std::net::SocketAddr;

use futures::compat::Future01CompatExt;

use tokio::codec::{FramedRead, FramedWrite};
use tokio::io::{AsyncRead, ReadHalf, WriteHalf};
use tokio::net::TcpStream;

use crate::codec::{GsbMessageDecoder, GsbMessageEncoder};

pub async fn connect(
    addr: &SocketAddr,
) -> (
    FramedRead<ReadHalf<TcpStream>, GsbMessageDecoder>,
    FramedWrite<WriteHalf<TcpStream>, GsbMessageEncoder>,
) {
    let sock = TcpStream::connect(&addr)
        .compat()
        .await
        .expect("Connect failed");
    let (reader, writer) = sock.split();
    let reader = FramedRead::new(reader, GsbMessageDecoder::new());
    let writer = FramedWrite::new(writer, GsbMessageEncoder {});
    (reader, writer)
}
