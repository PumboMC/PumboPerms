//! `config.yml`: the core sections and the Pumpkin-only `provider`.

use pumbo_common::config::{Check, Commands, Settings, Warning};
use pumbo_perms_core::config::{self as core_config, ContextCfg, CoreCfg, DefaultsCfg, ImportCfg, LimitsCfg, LogCfg};
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
    pub limits: LimitsCfg,
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
            limits: LimitsCfg::default(),
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
            limits: self.limits.clone(),
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
        self.limits = core.limits;
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

    /// A switch of the staff limits that is not `true` or `false`, a section
    /// that is not one and a misspelt switch leave the limits off, each with a
    /// warning.
    #[test]
    fn bad_limits_values_fall_back_to_off() {
        let (c, w) = config::load::<Config>("limits:\n  argument-permissions: on\n  require-group-membership: yes\n");
        assert_eq!(c.limits, LimitsCfg::default());
        let mut named: Vec<String> = w.iter().filter_map(|w| w.option.clone()).collect();
        named.sort();
        assert_eq!(named, ["limits.argument-permissions", "limits.require-group-membership"], "{w:?}");
        let (c, w) = config::load::<Config>("limits: true\n");
        assert_eq!((c.limits, w.len()), (LimitsCfg::default(), 1), "{w:?}");
        let (c, w) = config::load::<Config>("limits:\n  argument-permission: true\n");
        assert_eq!(c.limits, LimitsCfg::default());
        assert!(w.len() == 1 && w[0].message.contains("did you mean 'limits.argument-permissions'"), "{w:?}");
    }

    /// Start and `/pp reload` give the engine `Config::core()`.
    #[test]
    fn limits_from_the_file_reach_the_engine() {
        let (c, w) =
            config::load::<Config>("limits:\n  argument-permissions: true\n  require-group-membership: true\n");
        assert!(w.is_empty(), "{w:?}");
        let limits = c.core().limits;
        assert!(limits.argument_permissions && limits.require_group_membership);
        assert_eq!(Config::default().core().limits, LimitsCfg::default());
    }

    /// An update writes the new section into a `config.yml` nobody changed;
    /// a changed one stays, with a warning, and the limits stay off.
    #[test]
    fn edited_config_without_limits_keeps_them_off() {
        let start = TEMPLATE.find("# ── Staff limits").unwrap();
        let end = start + TEMPLATE[start..].find("\n\n\n").unwrap() + 3;
        let before = format!("{}{}", &TEMPLATE[..start], &TEMPLATE[end..]);
        assert!(!before.contains("limits"));
        let dir = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-limits-{}", std::process::id()));
        let path = dir.join("config.yml");
        let file = path.to_string_lossy().to_string();
        for (text, updated) in [(before.clone(), true), (before.replace("language: en", "language: pl"), false)] {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(".generated")).unwrap();
            std::fs::write(&path, &text).unwrap();
            std::fs::write(dir.join(".generated/config.yml"), &before).unwrap();
            let (read, notes) = config::read_or_create(&file, TEMPLATE);
            assert_eq!(read == TEMPLATE, updated, "{notes:?}");
            if !updated {
                assert!(notes.iter().any(|n| n.message.ends_with("their defaults apply: limits")), "{notes:?}");
            }
            let (c, w) = config::load::<Config>(&read);
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.core().limits, LimitsCfg::default());
        }
        let _ = std::fs::remove_dir_all(&dir);
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
