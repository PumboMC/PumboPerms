//! `/pp` on the proxy, the file the proxy sends, and exports.

use pumbo_common::clock::now_ms;
use pumbo_common::rich::Text as Rich;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, Action, Env, SourceMode, Who};
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::export;
use pumbo_perms_core::files;
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::ui;
use pumbo_sdk::contracts::PermissionsFile;
use pumbo_sdk::{PlayerId, QueryContext, log, permissions, players, scheduler};

use crate::setup;
use crate::{PumboPerms, SOURCE, State, json, reply, uuid_of, who_of};

impl PumboPerms {
    fn version_rows(s: &State) -> Vec<(String, String)> {
        let l = &s.engine.lang;
        let mode = if s.not_provider { l.get("mode-not-provider") } else { l.get("mode-provider") };
        vec![
            (l.get("version-platform"), "PumboProx".to_string()),
            (l.get("version-mode"), mode),
            (l.get("version-online"), s.loaded.len().to_string()),
            (l.get("version-servers"), s.engine.server_groups.len().to_string()),
        ]
    }

    /// Runs `/pp <args>` for a player (`Some`) or the console.
    pub(super) async fn run(&self, player: Option<PlayerId>, args: Vec<String>) {
        let who = match player {
            None => Who::console(),
            Some(id) => {
                let Some(p) = players::get(id) else { return };
                who_of(&p)
            }
        };
        let online = players::all();
        let now = now_ms();
        let dir = std::path::PathBuf::from(&self.data_dir);
        let mut reload = false;
        let result = self.with(|s| {
            let rows = Self::version_rows(s);
            let lookup = |name: &str| {
                online.iter().find(|p| p.profile.name.eq_ignore_ascii_case(name)).map(|p| KnownName {
                    uuid: uuid_of(p),
                    name: p.profile.name.clone(),
                    online: p.online_mode,
                })
            };
            let env = Env { now, online: &lookup, version: env!("CARGO_PKG_VERSION"), version_rows: rows };
            let out = command::run(&mut s.engine, &who, &args, &env);
            let mut text = out.reply;
            let mut changed = out.changed;
            let mut log_entry = out.log;
            match out.action {
                Some(Action::Reload) => reload = true,
                Some(Action::Export(name)) => text = export_file(&s.engine, &dir, &name, now).unwrap_or_else(|e| e),
                Some(Action::Import(name)) => {
                    let o = files::import_file(&mut s.engine, &dir, &who, &name, now);
                    text = o.reply;
                    changed |= o.changed;
                    log_entry = o.log.or(log_entry);
                }
                Some(Action::Source(source, mode)) => {
                    let l = &s.engine.lang;
                    let data = match (&s.file, source == SOURCE) {
                        (_, false) => Err(l.format("source-unknown", &Args::new().arg(&source).arg(SOURCE))),
                        (None, true) if mode != SourceMode::Undo => Err(l.get("source-no-file")),
                        (Some(Err(why)), true) if mode != SourceMode::Undo => {
                            Err(l.format("source-file-errors", &Args::new().arg(why)))
                        }
                        (f, true) => f.clone().and_then(Result::ok).ok_or_else(String::new),
                    };
                    let o = command::import_source(&mut s.engine, &who, &source, data, mode, now);
                    text = o.reply;
                    changed |= o.changed;
                }
                None => {}
            }
            let notice = log_entry.filter(|_| s.cfg.log.notify).map(|entry| {
                let line =
                    format!("PumboPerms: {} changed {} {}: {}", entry.actor, entry.kind, entry.target, entry.action);
                (command::notification(&s.engine, &entry), line)
            });
            (text, changed, notice)
        });
        let Some((mut text, changed, notice)) = result else {
            reply(player, &Rich::parse("&cPumboPerms is busy, try again."));
            return;
        };
        if reload {
            text = match self.reload() {
                Ok(t) => t,
                Err(e) => self
                    .with(|s| ui::error(&s.engine.lang, "command-reload-failed", &Args::new().arg(ui::value(&e))))
                    .unwrap_or_default(),
            };
        }
        reply(player, &text);
        if changed {
            self.push_all().await;
        }
        if let Some((message, line)) = notice {
            log::info(&line);
            for p in &online {
                if Some(p.id) != player && permissions::has(p.id, "pumbo.perms.log.notify", &QueryContext::Current) {
                    players::send_message(p.id, json(&message));
                }
            }
        }
    }

    /// `permissions.yml` from the proxy: imported at the first start (with
    /// `import.follow` again whenever it changes); the same file again does
    /// nothing. Reading a large file and importing it are two pieces of work
    /// with a pause between them.
    pub(super) async fn take_file(&self, f: PermissionsFile) {
        let snap = match export::parse(&f.data) {
            Ok(s) => s,
            Err(e) => {
                log::warn(&format!("PumboPerms: permissions.yml cannot be imported: {e}"));
                self.with(|s| s.file = Some(Err(e)));
                return;
            }
        };
        scheduler::sleep(0).await;
        let now = now_ms();
        let line = self
            .with(|s| {
                s.file = Some(Ok((f.fingerprint.clone(), snap.clone())));
                if !s.cfg.import.enabled {
                    return None;
                }
                let ledger = match s.engine.ledger(SOURCE) {
                    Ok(l) => l,
                    Err(why) => {
                        // An empty ledger would import everything again.
                        log::error(&format!("PumboPerms: permissions.yml is not imported automatically: {why}"));
                        return None;
                    }
                };
                if ledger.fingerprint == f.fingerprint {
                    return None;
                }
                if ledger.at != 0 && !s.cfg.import.follow {
                    return Some((
                        "changed since it was imported; /pp import preview file shows what it would add, /pp import run file imports it".to_string(),
                        false,
                    ));
                }
                let out = command::import_source(
                    &mut s.engine,
                    &Who::console(),
                    SOURCE,
                    Ok((f.fingerprint, snap)),
                    SourceMode::Run,
                    now,
                );
                Some((out.reply.plain(), out.changed))
            })
            .flatten();
        if let Some((text, changed)) = line {
            for l in text.lines() {
                log::info(&format!("permissions.yml: {l}"));
            }
            if changed {
                self.push_all().await;
            }
        }
    }
}

fn export_file(e: &Engine, dir: &std::path::Path, name: &str, now: u64) -> Result<Rich, Rich> {
    let path = files::export_to(e, dir, name, now)?;
    let shown = format!("plugins/data/{}/{path}", setup::PLUGIN_ID);
    Ok(ui::success(&e.lang, "exported", &Args::new().arg(ui::value(shown))))
}
