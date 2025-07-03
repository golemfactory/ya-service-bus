use clap::Parser;
use std::env;
use std::path::PathBuf;
use ya_sb_proto::{DEFAULT_GSB_URL, GSB_URL_ENV_VAR};
use ya_sb_router::{InstanceConfig, RouterConfig};

#[derive(Parser)]
#[command(
    about = "Service Bus Router with optional REST monitoring endpoints.\n\
If --web-addr is provided, a REST API with SSE events will be started for monitoring node state changes.\n\
If --web-addr is omitted, the REST API will not be started."
)]
struct Options {
    #[arg(short = 'l', env = GSB_URL_ENV_VAR, default_value = DEFAULT_GSB_URL)]
    gsb_url: url::Url,
    #[arg(long, default_value = "info")]
    log_level: String,
    /// How often send pings if there is no incoming packets.
    #[arg(long, default_value = "2min", env = "YA_SB_PING_INTERVAL")]
    ping_interval: humantime::Duration,
    #[arg(long, default_value = "5min", env = "YA_SB_PING_TIMEOUT")]
    ping_timeout: humantime::Duration,
    #[arg(long, default_value = "30s", env = "YA_SB_FW_TIMEOUT")]
    forward_timeout: humantime::Duration,
    #[arg(long, env = "YA_SB_GC")]
    gc_interval: Option<humantime::Duration>,
    #[arg(long, default_value = "8", env = "YA_SB_HIGH_BUFFER")]
    high_buffer_mark: usize,
    #[arg(long, short, default_value = "4", env = "YA_SB_BROADCAST_BACKLOG")]
    broadcast_backlog_max_size: usize,
    #[arg(long, env = "YA_SB_CERT")]
    cert: Option<PathBuf>,
    #[arg(long, env = "YA_SB_KEY")]
    key: Option<PathBuf>,
    /// Optional URL to bind REST monitoring API (e.g. http://127.0.0.1:8080). If not provided, REST API will not be started.
    #[arg(
        long = "web-addr",
        env = "YA_SB_WEB_ADDR",
        value_name = "URL",
        help = "Optional URL to bind REST monitoring API (e.g. http://127.0.0.1:8080). If not provided, REST API will not be started."
    )]
    web_addr: Option<url::Url>,
}

#[cfg(feature = "tls")]
mod tls {
    use crate::Options;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls_pemfile::{certs, ec_private_keys};
    use std::fs::File;
    use std::io;
    use std::io::BufReader;
    use std::path::Path;
    use ya_sb_router::RouterConfig;
    use ya_sb_util::tls::CertHash;

    fn load_certs(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
        let certs =
            certs(&mut BufReader::new(File::open(path)?)).collect::<io::Result<Vec<_>>>()?;
        for cert in &certs {
            log::info!("loaded cert {:?}", CertHash::from(cert))
        }
        Ok(certs)
    }

    fn load_keys(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
        ec_private_keys(&mut BufReader::new(File::open(path)?))
            .next()
            .unwrap()
            .map(Into::into)
    }

    pub fn add_config(options: &Options, config: &mut RouterConfig) -> anyhow::Result<()> {
        match (&options.cert, &options.key) {
            (Some(cert), Some(key)) => {
                let tls_config = rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(load_certs(cert)?, load_keys(key)?)?;
                config.tls = Some(tls_config)
            }
            (None, None) => (),
            _ => anyhow::bail!("misconfigured server expected key & cert"),
        }
        Ok(())
    }
}

#[actix_rt::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::parse();
    env::set_var(
        "RUST_LOG",
        env::var("RUST_LOG").unwrap_or(options.log_level.clone()),
    );
    env_logger::init();
    let mut config = RouterConfig::default();
    config.ping_interval = options.ping_interval.into();
    config.ping_timeout = options.ping_timeout.into();
    config.gc_interval = options.gc_interval.map(Into::into);
    config.high_buffer_mark = options.high_buffer_mark;
    config.forward_timeout = options.forward_timeout.into();
    config.broadcast_backlog = options.broadcast_backlog_max_size;

    #[cfg(feature = "tls")]
    tls::add_config(&options, &mut config)?;

    let instance_config = InstanceConfig::new(config);
    instance_config
        .run_router(Some(options.gsb_url), options.web_addr)
        .await?;

    Ok(())
}
