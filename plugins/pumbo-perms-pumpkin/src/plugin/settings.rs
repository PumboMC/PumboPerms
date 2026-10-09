//! Reading `config.yml` and `lang/<code>.yml`.

use pumbo_common::config::{self, Warning};
use pumbo_common::lang::Lang;
use pumbo_common::text::Args;
use pumbo_perms_core::ui::{self, Text};

use super::state::Rt;
use crate::BUNDLES;
use crate::config::{Config, TEMPLATE};

pub fn load_config(dir: &str) -> (Config, Vec<Warning>) {
    let (text, warning) = config::read_or_create(&format!("{dir}/config.yml"), TEMPLATE);
    let (cfg, mut warnings) = config::load::<Config>(&text);
    warnings.extend(warning);
    warnings.extend(config::old_files(dir));
    (cfg, warnings)
}

pub fn load_lang(dir: &str, code: &str) -> (Lang, Vec<Warning>) {
    let template = Lang::template(&BUNDLES, code);
    let (text, warning) = config::read_or_create(&format!("{dir}/lang/{code}.yml"), &template);
    let (lang, warnings) = Lang::load(&BUNDLES, code, Some(&text));
    let mut warnings: Vec<Warning> =
        warnings.into_iter().map(|w| Warning { message: format!("lang/{code}.yml: {}", w.message), ..w }).collect();
    warnings.extend(warning);
    (lang, warnings)
}

/// `/pp reload`: config and messages (the database stays as it is). Whether
/// the permission check event is answered only changes with a restart.
pub fn reload(rt: &mut Rt) -> Text {
    let (cfg, mut warnings) = load_config(&rt.dir);
    let (lang, more) = load_lang(&rt.dir, &cfg.language);
    warnings.extend(more);
    if let Some(w) = warnings.iter().find(|w| w.fatal) {
        rt.warn(format!("PumboPerms: reload refused, the current settings stay: {w}"));
        return ui::error(rt.lang(), "command-reload-failed", &Args::new().arg(ui::value(&w.message)));
    }
    if cfg.provider.check_event != rt.check_event {
        warnings.push(Warning::new(Some("provider.check-event"), "changes after a server restart"));
    }
    rt.engine.cfg = cfg.core();
    rt.engine.lang = lang;
    rt.cfg = cfg;
    for w in &warnings {
        rt.warn(format!("PumboPerms: config: {w}"));
    }
    if warnings.is_empty() {
        ui::success(rt.lang(), "command-reloaded", &Args::new())
    } else {
        ui::warn(rt.lang(), "reloaded-warnings", &Args::new().arg(ui::value(warnings.len())))
    }
}
