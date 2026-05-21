//! Time helpers. ChimpFlix stores all timestamps as Unix epoch
//! milliseconds (`i64`).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current time as Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
