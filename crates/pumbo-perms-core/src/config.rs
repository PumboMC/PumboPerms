//! The `context`, `defaults` and `log` sections of `config.yml`.

use pumbo_common::config::Check;
use serde::{Deserialize, Serialize};

use crate::context::{self, Contexts};

/// Where this server is, for `server=` and `group=` contexts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ContextCfg {
    /// Name of this server; nodes with `server=<name>` apply only here. Empty:
    /// such nodes never apply.
    pub server: String,
    /// Group of servers this one belongs to (`group=<name>`).
    pub group: String,
}

/// Answers for nodes nobody set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DefaultsCfg {
    /// Operators with at least this level get every `pumbo.*` node that no
    /// group or user decides (0: off). Other nodes keep the defaults of the
    /// platform.
    pub op_level: u8,
}

impl Default for DefaultsCfg {
    fn default() -> Self {
        Self { op_level: 3 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LogCfg {
    /// Changes kept in the log (0: unlimited).
    pub max_entries: u64,
    /// Tell online players with `pumbo.perms.log.notify` about every change.
    pub notify: bool,
}

impl Default for LogCfg {
    fn default() -> Self {
        Self { max_entries: 10_000, notify: true }
    }
}

/// The sections of the core, put into the platform's config struct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CoreCfg {
    pub context: ContextCfg,
    pub defaults: DefaultsCfg,
    pub log: LogCfg,
}

impl CoreCfg {
    pub fn validate(&mut self, check: &mut Check<'_>) {
        for (option, slot, key) in [
            ("context.server", &mut self.context.server, context::SERVER),
            ("context.group", &mut self.context.group, context::GROUP),
        ] {
            if slot.is_empty() {
                continue;
            }
            let lower = slot.to_lowercase();
            if Contexts::default().insert(key, &lower).is_err() {
                check.invalid(option, slot, &"");
                slot.clear();
            } else {
                *slot = lower;
            }
        }
        check.clamp("defaults.op-level", &mut self.defaults.op_level, 0, 4);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumbo_common::config::{self, Settings};

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default, rename_all = "kebab-case")]
    struct T {
        #[serde(flatten)]
        core: CoreCfg,
    }

    impl Settings for T {
        fn validate(&mut self, check: &mut Check<'_>) {
            self.core.validate(check);
        }
    }

    #[test]
    fn validates() {
        let (c, w) = config::load::<T>("context:\n  server: Lobby\n  group: bad value!\ndefaults:\n  op-level: 9\n");
        assert_eq!(c.core.context.server, "lobby");
        assert_eq!(c.core.context.group, "");
        assert_eq!(c.core.defaults.op_level, 4);
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = config::load::<T>("");
        assert!(w.is_empty());
        assert_eq!(c.core, CoreCfg::default());
    }
}
