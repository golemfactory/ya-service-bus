// Router Configuration

use std::env;
use std::env::VarError;
use std::time::Duration;

/// Message router configuration.
#[non_exhaustive]
pub struct RouterConfig {
    /// How often to check for incoming traffic for a connection.
    pub ping_interval: Duration,
    /// After what time a connection without incoming traffic is considered dead.
    pub ping_timeout: Duration,
    /// How many messages per topic should be store for lagging receivers.
    pub broadcast_backlog: usize,
    /// How much time to hold incoming data while waiting for a recipient.
    pub forward_timeout: Duration,
    /// How many messages can wait to be sent before server starts holding back senders.
    pub high_buffer_mark: usize,
    /// How often to scan for unused resources.
    pub gc_interval: Option<Duration>,
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
    /// New Configuration from environment variables.
    pub fn from_env() -> Self {
        env::var("GSB_PING_TIMEOUT")
            .and_then(|s| s.parse().map_err(|_| VarError::NotPresent))
            .map(|secs| RouterConfig {
                ping_timeout: Duration::from_secs(secs),
                ping_interval: Duration::from_secs(secs / 4),
                ..Default::default()
            })
            .unwrap_or_else(|_| Default::default())
    }

    /// Sets the number of seconds to scan for unused resources.
    pub fn gc_interval_secs(&mut self, gc_secs: u64) -> &mut Self {
        self.gc_interval = Some(Duration::from_secs(gc_secs));
        self
    }
}
