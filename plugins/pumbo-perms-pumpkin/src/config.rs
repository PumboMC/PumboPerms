//! `config.yml`: the core sections and the Pumpkin-only `provider`.

use pumbo_common::config::{Check, Commands, Settings, Warning};
use pumbo_perms_core::config::{self as core_config, ContextCfg, CoreCfg, DefaultsCfg, ImportCfg, LogCfg};
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
    pub commands: Commands,
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
            commands: core_config::default_commands(),
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
        Self { check_event: false, resend_commands: true, refresh_seconds: 5 }
    }
}

impl Config {
    pub fn core(&self) -> CoreCfg {
        CoreCfg {
            context: self.context.clone(),
            defaults: self.defaults.clone(),
            log: self.log.clone(),
            worlds: true,
            // Pumpkin does not tell whether Mojang checked an account.
            names_checked: false,
            disabled: self.commands.disabled.clone(),
        }
    }

    /// The short commands to register next to `/pumboperms`. Pumpkin's own
    /// commands and those of the other Pumbo plugins are skipped with a
    /// warning; which commands third-party plugins registered, the plugin API
    /// does not tell.
    pub fn aliases(&self) -> (Vec<String>, Vec<Warning>) {
        self.commands.aliases_to_register(crate::PLUGIN_NAME, |name| {
            let node = format!("minecraft:command.{name}");
            let builtin = pumbo_common::pumpkin::COMMAND_NODES.contains(&node.as_str())
                || pumbo_common::pumpkin::UNGUARDED_ALIASES.iter().any(|(alias, ..)| *alias == name);
            let owner = if builtin { Some("Pumpkin") } else { pumbo_common::pumpkin::pumbo_command_owner(name) };
            owner.map(str::to_string)
        })
    }
}

/// The command shown in help and clicks: the first short command, or the full name.
pub fn root(aliases: &[String]) -> String {
    format!("/{}", aliases.first().map_or(crate::PLUGIN_NAME, String::as_str))
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
        core_config::validate_commands(&mut self.commands, check);
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
        assert_eq!(c.provider.refresh_seconds, 1, "clamped up from 0");
        assert_eq!(c.defaults.op_level, 4);
        assert_eq!(w.len(), 3, "{w:?}");
        let (c, w) = config::load::<Config>("language: \"bad lang!\"\n");
        assert_eq!(c.language, "en");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn aliases_come_from_config() {
        let (c, w) = config::load::<Config>("commands:\n  aliases: [perms, pp]\n  disabled: [u, editor, bogus]\n");
        assert_eq!(c.aliases(), (vec!["perms".to_string(), "pp".to_string()], Vec::new()));
        assert_eq!(root(&c.aliases().0), "/perms");
        assert_eq!(c.core().disabled, vec!["user", "editor"]);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].message.contains("'bogus' is not a /pp subcommand"), "{w:?}");
        // the defaults: /pp next to /pumboperms
        assert_eq!(Config::default().aliases().0, vec!["pp"]);
    }

    #[test]
    fn cleared_alias_list_registers_only_full_name() {
        let (c, _) = config::load::<Config>("commands:\n  aliases: []\n");
        let (aliases, w) = c.aliases();
        assert!(aliases.is_empty() && w.is_empty());
        assert_eq!(root(&aliases), "/pumboperms");
    }

    #[test]
    fn taken_alias_is_skipped_with_warning() {
        let (c, _) = config::load::<Config>("commands:\n  aliases: [ban, tp, login, pf, pp]\n");
        let (aliases, w) = c.aliases();
        assert_eq!(aliases, vec!["pp"]);
        let w: Vec<String> = w.into_iter().map(|w| w.message).collect();
        assert_eq!(
            w,
            vec![
                "/ban belongs to Pumpkin, not registered; /pumboperms works",
                "/tp belongs to Pumpkin, not registered; /pumboperms works",
                "/login belongs to PumboAuth, not registered; /pumboperms works",
                "/pf belongs to PumboFilter, not registered; /pumboperms works",
            ]
        );
    }
}
