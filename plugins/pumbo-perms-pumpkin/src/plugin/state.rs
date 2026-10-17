//! The plugin state ([`crate::state`]) and its log lines, written to Pumpkin.

pub use crate::state::*;

/// Writes the log lines collected inside [`with`], with the engine's warnings.
pub fn flush_logs() {
    let (lines, errors) = with(|rt| {
        let warnings = rt.engine.take_warnings();
        rt.logs.extend(warnings.into_iter().map(|line| (true, line)));
        (std::mem::take(&mut rt.logs), std::mem::take(&mut rt.errors))
    })
    .unwrap_or_default();
    for line in errors {
        super::api::error(&line);
    }
    for (warn, line) in lines {
        if warn {
            super::api::warn(&line);
        } else {
            super::api::info(&line);
        }
    }
}
