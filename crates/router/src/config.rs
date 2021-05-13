// Router Configuration

use std::env;
use std::env::VarError;
use std::time::Duration;

pub struct RouterConfig {
    pub(crate) ping_timeout: Duration,
    pub(crate) ping_interval: Duration,
    pub(crate) broadcast_backlog: usize,
    pub(crate) gc_interval: Option<Duration>,
    pub(crate) forward_timeout: Duration,
    pub(crate) high_buffer_mark: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        RouterConfig {
            ping_timeout: Duration::from_secs(20),
            ping_interval: Duration::from_secs(10),
            forward_timeout: Duration::from_secs(60),
            broadcast_backlog: 16,
            gc_interval: None,
            high_buffer_mark: 16,
        }
    }
}

impl RouterConfig {
    pub fn from_env() -> Self {
        env::var("GSB_PING_TIMEOUT")
            .and_then(|s| s.parse().map_err(|_| VarError::NotPresent))
            .and_then(|secs| {
                Ok(RouterConfig {
                    ping_timeout: Duration::from_secs(secs),
                    ping_interval: Duration::from_secs(secs / 4),
                    ..Default::default()
                })
            })
            .unwrap_or_else(|_| Default::default())
    }

    #[inline]
    pub fn gc_interval_secs(&mut self, secs: u64) -> &mut Self {
        self.gc_interval = Some(Duration::from_secs(secs));
        self
    }

    pub fn broadcast_backlog(&mut self, backlog: usize) -> &mut Self {
        self.broadcast_backlog = backlog;
        self
    }
}
