use tokio;
use structopt::StructOpt;
use dotenv;
use env_logger;

#[derive(Debug, StructOpt)]
#[structopt(name = "options", about = "Options for data limiter relay")]
struct Opt {
    // we don't want to name it "speed", need to look smart
    #[structopt(default_value = "127.0.0.1:111")]
    target_addr: String,
}


async fn run() -> anyhow::Result<()> {
    let opt = Opt::from_args();
    dotenv::dotenv().ok();

    log::debug!("Starting relay. Target addr: {}", opt.target_addr);

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();

    match run().await {
        Ok(()) => {

        }
        Err(err) => {

        }
    }

}
