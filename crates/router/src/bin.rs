use std::env;
use structopt::{clap, StructOpt};
use ya_sb_proto::{DEFAULT_GSB_URL, GSB_URL_ENV_VAR};

#[derive(StructOpt)]
#[structopt(about = "Service Bus Router")]
#[structopt(global_setting = clap::AppSettings::ColoredHelp)]
struct Options {
    #[structopt(short = "l", env = GSB_URL_ENV_VAR, default_value = DEFAULT_GSB_URL)]
    gsb_url: url::Url,
    #[structopt(long, default_value = "debug")]
    log_level: String,
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::from_args();
    env::set_var(
        "RUST_LOG",
        env::var("RUST_LOG").unwrap_or(options.log_level),
    );
    env_logger::init();

    ya_sb_router::bind_gsb_router(Some(options.gsb_url)).await?;
    tokio::signal::ctrl_c().await?;
    println!();
    log::info!("SIGINT received, exiting");
    Ok(())
}
