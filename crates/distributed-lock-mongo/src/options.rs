use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MongoDistributedLockOptions {
    pub expiry: Duration,
    pub min_busy_wait_sleep_time: Duration,
    pub max_busy_wait_sleep_time: Duration,
    pub extension_cadence: Duration,
}

impl Default for MongoDistributedLockOptions {
    fn default() -> Self {
        Self {
            expiry: Duration::from_secs(30),
            min_busy_wait_sleep_time: Duration::from_millis(10),
            max_busy_wait_sleep_time: Duration::from_secs(1),
            extension_cadence: Duration::from_secs(10),
        }
    }
}
