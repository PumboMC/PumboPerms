//! The plugin state ([`crate::state`]) and its log lines, written to Pumpkin.

pub use crate::state::*;

/// Writes the log lines collected inside [`with`].
pub fn flush_logs() {
    let lines = with(|rt| std::mem::take(&mut rt.logs)).unwrap_or_default();
    for (warn, line) in lines {
        if warn {
            super::api::warn(&line);
        } else {
            super::api::info(&line);
        }
    }
}
