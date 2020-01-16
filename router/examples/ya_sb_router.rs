use std::env;
use structopt::StructOpt;
use ya_service_api::constants::YAGNA_BUS_ADDR_STR;

#[derive(StructOpt)]
#[structopt(name = "Router", about = "Service Bus Router")]
struct Options {
    #[structopt(short = "l", default_value = &YAGNA_BUS_ADDR_STR)]
    ip_port: String,
    #[structopt(long, default_value = "debug")]
    log_level: String,
}

#[tokio::main]
async fn main() -> failure::Fallible<()> {
    let options = Options::from_args();
    env::set_var(
        "RUST_LOG",
        env::var("RUST_LOG").unwrap_or(options.log_level),
    );
    env_logger::init();
    let listen_addr = options.ip_port.parse().expect("Invalid ip:port");

    ya_sb_router::bind_router(listen_addr).await?;
    tokio::signal::ctrl_c().await?;
    println!();
    log::info!("SIGINT received, exiting");
    Ok(())
}
