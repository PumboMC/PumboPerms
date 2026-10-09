//! `config.yml`: the core sections and the Pumpkin-only `provider`.

use pumbo_common::config::{Check, Settings};
use pumbo_perms_core::config::{ContextCfg, CoreCfg, DefaultsCfg, LogCfg};
use serde::{Deserialize, Serialize};

/// The file written at the first start.
pub const TEMPLATE: &str = include_str!("../assets/config.yml");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub language: String,
    pub context: ContextCfg,
    pub defaults: DefaultsCfg,
    pub log: LogCfg,
    pub provider: ProviderCfg,
    pub import: ImportCfg,
}

/// Taking over other permission sources (PumboBridge spec §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImportCfg {
    /// At start, import the proxy's table (through PumboBridge), the plugins
    /// below, or else the attachments other plugins set.
    pub enabled: bool,
    /// Import a source again whenever it changes (otherwise once).
    pub follow: bool,
    /// Other plugins that answer `{"op":"export-permissions"}`.
    pub plugins: Vec<String>,
}

impl Default for ImportCfg {
    fn default() -> Self {
        Self { enabled: true, follow: false, plugins: Vec::new() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "en".into(),
            context: ContextCfg::default(),
            defaults: DefaultsCfg::default(),
            log: LogCfg::default(),
            provider: ProviderCfg::default(),
            import: ImportCfg::default(),
        }
    }
}

/// How Pumpkin learns the decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProviderCfg {
    /// Also answer Pumpkin's permission check event (every node, even ones
    /// nobody named, with full wildcard and context rules). Unsafe on Pumpkin
    /// 0.2.0 and 0.1.0-dev, see the README: the server hangs when a plugin
    /// registers commands while players are online and under heavy load.
    pub check_event: bool,
    /// Send players their command list again after their permissions change,
    /// so that Tab completion shows exactly what they may use.
    pub resend_commands: bool,
    /// Seconds between checks for expired nodes and changed operator levels.
    pub refresh_seconds: u32,
}

impl Default for ProviderCfg {
    fn default() -> Self {
        Self { check_event: false, resend_commands: true, refresh_seconds: 1 }
    }
}

impl Config {
    pub fn core(&self) -> CoreCfg {
        CoreCfg { context: self.context.clone(), defaults: self.defaults.clone(), log: self.log.clone() }
    }
}

impl Settings for Config {
    fn validate(&mut self, check: &mut Check<'_>) {
        let lang = self.language.trim().to_lowercase();
        check.ensure("language", &mut self.language, "en".into(), |_| {
            !lang.is_empty()
                && lang.len() <= 16
                && lang.bytes().all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'_')
        });
        self.language = self.language.trim().to_lowercase();
        let mut core = self.core();
        core.validate(check);
        self.context = core.context;
        self.defaults = core.defaults;
        self.log = core.log;
        check.clamp("provider.refresh-seconds", &mut self.provider.refresh_seconds, 1, 3600);
        let plugins = self.import.plugins.clone();
        check.ensure("import.plugins", &mut self.import.plugins, Vec::new(), |_| {
            plugins.iter().all(|p| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumbo_common::config;

    #[test]
    fn template_gives_exactly_the_defaults() {
        let (c, w) = config::load::<Config>(TEMPLATE);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn bad_values_fall_back() {
        let (c, w) = config::load::<Config>(
            "language: PL\nprovider:\n  check-event: true\n  refresh-seconds: 0\ndefaults:\n  op-level: 7\n  unknown: 1\n",
        );
        assert_eq!(c.language, "pl");
        assert!(c.provider.check_event);
        assert_eq!(c.provider.refresh_seconds, 1);
        assert_eq!(c.defaults.op_level, 4);
        assert_eq!(w.len(), 3, "{w:?}");
        let (c, w) = config::load::<Config>("language: \"bad lang!\"\n");
        assert_eq!(c.language, "en");
        assert_eq!(w.len(), 1);
    }
}
