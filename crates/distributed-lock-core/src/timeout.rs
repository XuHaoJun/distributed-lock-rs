//! Timeout value helpers.

use std::time::Duration;

/// Represents a timeout duration for lock operations.
/// 
/// - `Some(duration)` - Wait up to this duration
/// - `None` - Wait indefinitely
pub type Timeout = Option<Duration>;

/// Internal helper for timeout calculations.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutValue {
    millis: i64, // -1 for infinite
}

impl TimeoutValue {
    pub const INFINITE: Self = Self { millis: -1 };
    pub const ZERO: Self = Self { millis: 0 };
    
    pub fn is_infinite(&self) -> bool {
        self.millis < 0
    }
    
    pub fn is_zero(&self) -> bool {
        self.millis == 0
    }
    
    pub fn as_duration(&self) -> Option<Duration> {
        if self.is_infinite() {
            None
        } else {
            Some(Duration::from_millis(self.millis as u64))
        }
    }
}

impl From<Option<Duration>> for TimeoutValue {
    fn from(timeout: Option<Duration>) -> Self {
        match timeout {
            None => Self::INFINITE,
            Some(d) => Self {
                millis: d.as_millis() as i64,
            },
        }
    }
}
