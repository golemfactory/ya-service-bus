use dotenv;
use env_logger;
use futures::future::{AbortHandle, Abortable};
use structopt::StructOpt;
use tokio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn process_socket(socket: TcpStream, target: TcpStream) -> anyhow::Result<()> {
    // do work with socket here
    let (mut rd, mut wr) = tokio::io::split(socket);
    let (mut rdt, mut wrt) = tokio::io::split(target);
    let (abort_handle, abort_registration) = AbortHandle::new_pair();
    let (abort_handle2, abort_registration2) = AbortHandle::new_pair();

    tokio::spawn(async move {
        let reader_future = Abortable::new(
            async move {
                let mut buf = vec![0; 16];

                loop {
                    let n = match rd.read(&mut buf).await {
                        Ok(n) => n,
                        Err(err) => {
                            log::error!("Error when reading {:?}", err);
                            break;
                        }
                    };

                    if n == 0 {
                        log::error!("Read 0 bytes");
                        break;
                    }
                    println!("GOT {:?}", &buf[..n]);
                    wrt.write_all(&buf[..n]).await.unwrap();
                }
                abort_handle2.abort();
            },
            abort_registration,
        );
        match reader_future.await {
            Ok(()) => {
                log::error!("Reader part of p9 communication ended too soon");
            }
            Err(e) => {
                log::info!("Future aborted, reason {e}");
            }
        }
    });
    let writer_future = Abortable::new(
        async move {
            let mut buf = vec![0; 16];

            loop {
                let n = match rdt.read(&mut buf).await {
                    Ok(n) => n,
                    Err(err) => {
                        log::error!("Error when reading {:?}", err);
                        break;
                    }
                };

                if n == 0 {
                    log::error!("Read 0 bytes");
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]);
                println!("GOT from server {:?}", s);
                wr.write_all(&buf[..n]).await.unwrap();
            }
            abort_handle.abort();
        },
        abort_registration2,
    );

    match writer_future.await {
        Ok(()) => {
            log::error!("Reader part of p9 communication ended too soon");
        }
        Err(e) => {
            log::info!("Future aborted, reason {e}");
        }
    }

    Ok(())
}

#[derive(Debug, StructOpt)]
#[structopt(name = "options", about = "Options for data limiter relay")]
struct Opt {
    // we don't want to name it "speed", need to look smart
    #[structopt(long = "--listen-addr", default_value = "127.0.0.1:22444")]
    listen_addr: String,
    // we don't want to name it "speed", need to look smart
    #[structopt(long = "--target-addr", default_value = "127.0.0.1:17777")]
    target_addr: String,
}

async fn run() -> anyhow::Result<()> {
    let opt = Opt::from_args();
    dotenv::dotenv().ok();

    log::debug!(
        "Starting relay. Listen addr: {}, target addr: {}",
        opt.listen_addr,
        opt.target_addr
    );

    let listener = TcpListener::bind(opt.listen_addr).await?;
    let (socket, _) = listener.accept().await?;
    let connection = TcpStream::connect(opt.target_addr).await?;
    process_socket(socket, connection).await?;
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();

    match run().await {
        Ok(()) => {
            log::info!("Relay ended successfully");
        }
        Err(err) => {
            log::error!("Relay ended with err {:?}", err);
        }
    }
}
