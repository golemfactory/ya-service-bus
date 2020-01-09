use structopt::StructOpt;

#[derive(StructOpt)]
#[structopt(name = "Router", about = "Service Bus Router")]
struct Options {
    #[structopt(short = "l", default_value = "127.0.0.1:8245")]
    ip_port: String,
    #[structopt(default_value = "debug")]
    log_level: String,
}

#[tokio::main]
async fn main() -> failure::Fallible<()> {
    flexi_logger::Logger::with_env_or_str("info")
        .start()
        .unwrap();

    let options = Options::from_args();
    let listen_addr = options.ip_port.parse().expect("Invalid ip:port");

    flexi_logger::Logger::with_env_or_str(format!("ya_sb_router={},info", options.log_level))
        .start()
        .unwrap();
    ya_sb_router::bind_router(listen_addr).await
}
