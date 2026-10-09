//! Host-independent parts of the PumboProx layer: the config, the message
//! files and the permission sets handed to the proxy.

use pumbo_common::config::{self, Check, Settings, Warning};
use pumbo_common::lang::{Bundle, COMMON, Lang};
use pumbo_perms_core::config::{CoreCfg, DefaultsCfg, LogCfg};
use pumbo_perms_core::engine::Engine;
use serde::{Deserialize, Serialize};

/// Plugin id in the manifest.
pub const PLUGIN_ID: &str = "pumbo-perms";

/// The default `config.yml` (built into the `.wasm`, written by the proxy).
pub const CONFIG_TEMPLATE: &str = include_str!("../assets/config.yml");

/// Messages of the PumboProx layer (prefix, version rows).
pub const LANG: Bundle = Bundle {
    name: "perms-prox",
    files: &[("en", include_str!("../lang/en.yml")), ("pl", include_str!("../lang/pl.yml"))],
};

/// Every message bundle, in load order.
pub const BUNDLES: [Bundle; 3] = [COMMON, pumbo_perms_core::LANG, LANG];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub language: String,
    pub log: LogCfg,
    pub import: ImportCfg,
}

/// Taking over the proxy's `permissions.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ImportCfg {
    /// Import `permissions.yml` at the first start.
    pub enabled: bool,
    /// Import it again whenever it changes (otherwise once).
    pub follow: bool,
}

impl Default for ImportCfg {
    fn default() -> Self {
        Self { enabled: true, follow: false }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { language: "en".into(), log: LogCfg::default(), import: ImportCfg::default() }
    }
}

impl Config {
    /// The core sections: places come from the player's server and the proxy
    /// has no operator levels.
    pub fn core(&self) -> CoreCfg {
        CoreCfg { defaults: DefaultsCfg { op_level: 0 }, log: self.log.clone(), ..CoreCfg::default() }
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
    }
}

/// Reads `config.yml` from the read-only config folder (`/config` in the
/// plugin, written by the proxy at the first start); missing means defaults.
pub fn load_config(dir: &str) -> (Config, Vec<Warning>) {
    let (cfg, mut w) = match std::fs::read_to_string(format!("{dir}/config.yml")) {
        Ok(text) => config::load::<Config>(&text),
        Err(_) => (Config::default(), Vec::new()),
    };
    w.extend(config::old_files(dir));
    (cfg, w)
}

/// The messages in `code`, with `lang/<code>.yml` from the config folder over
/// the built-in ones.
pub fn load_lang(dir: &str, code: &str) -> (Lang, Vec<Warning>) {
    let user = std::fs::read_to_string(format!("{dir}/lang/{code}.yml")).ok();
    let (lang, w) = Lang::load(&BUNDLES, code, user.as_deref());
    (lang, w.into_iter().map(|w| Warning { message: format!("lang/{code}.yml: {}", w.message), ..w }).collect())
}

/// What the proxy gets for one player: decisions in the global context and,
/// for every server whose decisions differ, in that server's context (the
/// proxy falls back to global where a server has none). `servers` come from
/// `engine.server_groups`.
pub fn provider_set(e: &mut Engine, uuid: &str, now: u64) -> Vec<(Option<String>, String, bool)> {
    let global = pumbo_perms_core::host::decisions(e, uuid, "", now);
    let servers: Vec<String> = e.server_groups.keys().cloned().collect();
    let mut out: Vec<(Option<String>, String, bool)> = Vec::new();
    for server in servers {
        let here = pumbo_perms_core::host::decisions(e, uuid, &server, now);
        // Equal everywhere: the global entries answer for this server too.
        if here != global {
            out.extend(here.into_iter().map(|(n, v)| (Some(server.clone()), n, v)));
        }
    }
    out.extend(global.into_iter().map(|(n, v)| (None, n, v)));
    out
}
