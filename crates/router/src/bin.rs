use std::env;
use structopt::{clap, StructOpt};
use ya_sb_proto::{DEFAULT_GSB_URL, GSB_URL_ENV_VAR};
use ya_sb_router::{InstanceConfig, RouterConfig};

#[derive(StructOpt)]
#[structopt(about = "Service Bus Router")]
#[structopt(global_setting = clap::AppSettings::ColoredHelp)]
struct Options {
    #[structopt(short = "l", env = GSB_URL_ENV_VAR, default_value = DEFAULT_GSB_URL)]
    gsb_url: url::Url,
    #[structopt(long, default_value = "info")]
    log_level: String,
    /// How often send pings if there is no incoming packets.
    #[structopt(long, default_value = "2min", env = "YA_SB_PING_INTERVAL")]
    ping_interval: humantime::Duration,
    #[structopt(long, default_value = "5min", env = "YA_SB_PING_TIMEOUT")]
    ping_timeout: humantime::Duration,
    #[structopt(long, env = "YA_SB_GC")]
    gc_interval: Option<humantime::Duration>,
    #[structopt(long, short, default_value = "8", env = "YA_SB_HIGH_BUFFER")]
    high_buffer_mark: usize,
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::from_args();
    env::set_var(
        "RUST_LOG",
        env::var("RUST_LOG").unwrap_or(options.log_level),
    );
    env_logger::init();
    let mut config = RouterConfig::default();
    config.ping_interval = options.ping_interval.into();
    config.ping_timeout = options.ping_timeout.into();
    config.gc_interval = options.gc_interval.map(Into::into);
    config.high_buffer_mark = options.high_buffer_mark;

    InstanceConfig::new(config)
        .run_url(Some(options.gsb_url))
        .await?;

    Ok(())
}
