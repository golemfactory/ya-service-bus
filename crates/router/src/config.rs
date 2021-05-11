// Router Configuration

use std::time::Duration;
use std::env;
use std::env::VarError;


pub struct RouterConfig {
    ping_timeout: Duration,
    ping_interval: Duration,
    broadcast_backlog : usize,
    gc_interval : Option<Duration>
}

impl Default for RouterConfig {
    fn default() -> Self {
        RouterConfig {
            ping_timeout: Duration::seconds(20),
            ping_interval: Duration::seconds(10),
            broadcast_backlog: 16,
            gc_interval: None
        }
    }
}

impl RouterConfig {
    pub fn from_env() -> Self {
        env::var("GSB_PING_TIMEOUT")
            .and_then(|s| s.parse().map_err(|_| VarError::NotPresent))
            .and_then(|secs| {
                Ok(RouterConfig {
                    ping_timeout: Duration::seconds(secs),
                    ping_interval: Duration::seconds(secs / 4),
                    .. Default::default()
                })
            })
            .unwrap_or_else(|_| Default::default())
    }

    #[inline]
    pub fn gc_interval_secs(&mut self, secs : u64) -> &mut Self {
        self.gc_interval = Some(Duration::from_secs(secs));
        self
    }

    pub fn broadcast_backlog(&mut self, backlog : usize) -> &mut Self {
        self.broadcast_backlog = backlog;
        self
    }
}
