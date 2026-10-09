//! Taking over other permission sources (PumboBridge spec §5.3): the
//! proxy's table through PumboBridge (`proxy`), plugins named in
//! `import.plugins`, or else the attachments other plugins set
//! (`attachments`). A local PumboPerms has precedence over the bridge: after
//! the first pass it asks the bridge to release the attachments and writes
//! them alone.
//!
//! When the proxy's PumboPerms rules the network (the bridge's copy of its
//! export says `proxy-rules`), this plugin steps back instead: it hands its
//! attachments to PumboBridge (`take-permissions`), which writes the proxy's
//! decisions, and takes over again when the proxy has no PumboPerms any more
//! (PumboPerms spec §16).
//!
//! What each source added is kept in `imports/<source>.json` (its ledger),
//! so a later import replaces only that and `/pp import undo` takes it back.
//! Every call to another plugin happens outside the state (`ipc` may run
//! that plugin's handler inside it).

use pumbo_common::clock::now_ms;
use pumbo_perms_core::command::{self, Outcome, SourceMode, Who};
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::merge::{self, Ledger};

use super::api::{self, Server};
use super::push;
use super::state::{self, Rt};

/// The bridge plugin on Pumpkin.
const BRIDGE: &str = "pumbobridge";
/// How long a "pending" source is asked again after the start.
const WAIT_MS: u64 = 120_000;

/// One source's answer to `{"op":"export-permissions"}`.
enum Answer {
    Data(String, Snapshot),
    Pending,
    /// No such plugin (or it does not export).
    Absent,
    Bad(String),
}

fn ask(plugin: &str) -> Answer {
    let Ok(reply) = crate::papi::ipc::send_ipc_message(plugin, br#"{"op":"export-permissions"}"#) else {
        return Answer::Absent;
    };
    let Ok(bytes) = reply else { return Answer::Absent };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Answer::Bad("not JSON".into());
    };
    if v.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return if v.get("pending") == Some(&serde_json::Value::Bool(true)) { Answer::Pending } else { Answer::Absent };
    }
    let fingerprint = v.get("fingerprint").and_then(|f| f.as_str()).unwrap_or_default().to_string();
    let data = v.get("data").map(|d| d.to_string()).unwrap_or_default();
    match export::parse(&data) {
        Ok(snap) => Answer::Data(fingerprint, snap),
        Err(e) => Answer::Bad(e),
    }
}

/// Attachments other plugins set on online players (Pumpkin's command nodes),
/// as user entries.
fn attachments(server: &Server) -> Option<(String, Snapshot)> {
    let mut users = Vec::new();
    for p in server.get_all_players() {
        let permissions: Vec<pumbo_perms_core::model::PermNode> = pumbo_common::pumpkin::COMMAND_NODES
            .iter()
            .filter_map(|n| {
                p.has_permission_set(n).map(|value| pumbo_perms_core::model::PermNode {
                    node: (*n).to_string(),
                    value,
                    context: Default::default(),
                    expiry: None,
                })
            })
            .collect();
        if !permissions.is_empty() {
            users.push(pumbo_perms_core::model::User {
                uuid: api::uuid_string(&p.get_id()),
                name: p.get_name(),
                data: pumbo_perms_core::model::Data { permissions, ..Default::default() },
            });
        }
    }
    if users.is_empty() {
        return None;
    }
    let snap = Snapshot {
        format: export::FORMAT.into(),
        version: export::VERSION,
        exported: now_ms(),
        groups: Vec::new(),
        users,
        tracks: Vec::new(),
        by_name: Vec::new(),
        proxy_rules: false,
    };
    let text = serde_json::to_string(&snap).unwrap_or_default();
    Some((merge::fingerprint(&text), snap))
}

fn ledger_path(dir: &str, source: &str) -> String {
    format!("{dir}/imports/{source}.json")
}

fn load_ledger(dir: &str, source: &str) -> Ledger {
    std::fs::read_to_string(ledger_path(dir, source))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_ledger(rt: &mut Rt, source: &str, ledger: &Ledger) {
    let path = ledger_path(&rt.dir, source);
    let result = if ledger.entries.is_empty() && ledger.fingerprint.is_empty() {
        std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
    } else {
        let _ = std::fs::create_dir_all(format!("{}/imports", rt.dir));
        std::fs::write(&path, serde_json::to_string_pretty(ledger).unwrap_or_default())
    };
    if let Err(e) = result {
        rt.warn(format!("PumboPerms: cannot save {path}: {e}"));
    }
}

/// The plugin asked for a source name (`proxy` = PumboBridge).
fn plugin_of(source: &str) -> &str {
    if source == "proxy" { BRIDGE } else { source }
}

fn fetch(server: &Server, source: &str) -> Result<(String, Snapshot), String> {
    if source == "attachments" {
        return attachments(server).ok_or_else(|| "no attachments of other plugins on online players".into());
    }
    match ask(plugin_of(source)) {
        Answer::Data(f, s) => Ok((f, s)),
        Answer::Pending => Err("the source has no data yet (is the proxy connected?)".into()),
        Answer::Absent => Err(format!("no plugin {} answers export-permissions", plugin_of(source))),
        Answer::Bad(e) => Err(e),
    }
}

/// `/pp import run|preview|undo <source>` (runs inside the state: `ipc`
/// into another plugin cannot come back here, the state is busy then).
pub fn command(rt: &mut Rt, server: &Server, who: &Who, source: &str, mode: SourceMode, now: u64) -> Outcome {
    let data = if mode == SourceMode::Undo { Err(String::new()) } else { fetch(server, source) };
    let ledger = load_ledger(&rt.dir, source);
    let (out, new) = command::import_source(&mut rt.engine, who, source, data, &ledger, mode, now);
    if let Some(l) = new {
        save_ledger(rt, source, &l);
    }
    out
}

/// The proxy's PumboPerms rules (`on`) or not any more: steps back or takes
/// over again.
fn proxy_rules(on: bool) {
    if !state::set_passive(on) {
        return;
    }
    if on {
        // Everything this plugin set goes to the bridge, which unsets it
        // (offline players at their join) and writes the proxy's decisions.
        let nodes: std::collections::BTreeMap<String, Vec<String>> =
            state::with(|rt| rt.pushed.drain().map(|(uuid, nodes)| (uuid, nodes.into_keys().collect())).collect())
                .unwrap_or_default();
        let msg = serde_json::json!({"op": "take-permissions", "nodes": nodes}).to_string();
        let taken = crate::papi::ipc::send_ipc_message(BRIDGE, msg.as_bytes()).is_ok();
        api::info(&format!(
            "PumboPerms: the proxy's PumboPerms rules this network: this plugin stepped back{}; set ranks with /pp on the proxy",
            if taken { ", PumboBridge writes the permissions" } else { "" }
        ));
    } else {
        // The first pass again: release the bridge and write everything.
        state::with(|rt| rt.ready = false);
        api::info(
            "PumboPerms: the proxy has no PumboPerms any more; this plugin takes over the permissions here again",
        );
    }
}

/// Every 10 s from the first tick: whether the proxy's PumboPerms rules, the
/// first import pass, then (with `import.follow`) new versions of the sources.
pub fn tick(server: &Server) {
    let now = now_ms();
    let mut bridge = Some(ask(BRIDGE));
    if let Some(Answer::Data(_, snap)) = &bridge {
        proxy_rules(snap.proxy_rules);
    }
    if state::passive() {
        return;
    }
    let Some((cfg, dir, ready, waiting_until)) =
        state::with(|rt| (rt.cfg.import.clone(), rt.dir.clone(), rt.ready, rt.waiting_until))
    else {
        return;
    };
    if ready && !cfg.follow {
        match waiting_until {
            Some(t) if now < t => {}
            Some(_) => {
                state::with(|rt| {
                    rt.waiting_until = None;
                    rt.warn("PumboPerms: the proxy's permission table did not come within 2 minutes; run /pp import run proxy once the proxy is connected".into());
                });
                state::flush_logs();
                return;
            }
            None => return,
        }
    }
    let mut answers = Vec::new();
    if cfg.enabled {
        let sources = std::iter::once("proxy".to_string()).chain(cfg.plugins.iter().cloned());
        for source in sources {
            let answer = match bridge.take().filter(|_| source == "proxy") {
                Some(a) => a,
                None => ask(plugin_of(&source)),
            };
            answers.push((source, answer));
        }
    }
    let pending = answers.iter().any(|(_, a)| matches!(a, Answer::Pending));
    let any_data = answers.iter().any(|(_, a)| matches!(a, Answer::Data(..)));
    let fallback = if cfg.enabled && !ready && !any_data && !pending && load_ledger(&dir, "attachments").at == 0 {
        attachments(server)
    } else {
        None
    };
    let result = state::with(|rt| {
        let mut changed = false;
        let who = Who::console();
        let mut runs: Vec<(String, String, Snapshot)> = Vec::new();
        for (source, answer) in answers {
            match answer {
                Answer::Data(f, s) => runs.push((source, f, s)),
                Answer::Bad(e) => rt.warn(format!("PumboPerms: cannot read permissions of {source}: {e}")),
                Answer::Pending | Answer::Absent => {}
            }
        }
        if let Some((f, s)) = fallback {
            runs.push(("attachments".into(), f, s));
        }
        for (source, fingerprint, snap) in runs {
            let ledger = load_ledger(&rt.dir, &source);
            // Once per source unless `follow` (a ledger means it was imported).
            if ledger.at != 0 && !cfg.follow {
                continue;
            }
            let (out, new) = command::import_source(
                &mut rt.engine,
                &who,
                &source,
                Ok((fingerprint, snap)),
                &ledger,
                SourceMode::Run,
                now,
            );
            if let Some(l) = new {
                save_ledger(rt, &source, &l);
                changed = true;
                rt.logs.push((false, out.reply.plain()));
            }
        }
        if pending && !rt.ready {
            rt.waiting_until = Some(now + WAIT_MS);
        } else if !pending {
            rt.waiting_until = None;
        }
        let first = !rt.ready;
        rt.ready = true;
        if changed || first {
            rt.refresh_catalog();
        }
        let plans = if changed || first { rt.plan_all(now) } else { Vec::new() };
        (first, plans, rt.resend())
    });
    state::flush_logs();
    let Some((first, plans, resend)) = result else { return };
    if first {
        // The bridge (if any) takes its attachments back before ours go on.
        let released = crate::papi::ipc::send_ipc_message(BRIDGE, br#"{"op":"release-permissions"}"#).is_ok();
        if released {
            api::info("PumboPerms: took over the permissions from PumboBridge");
        }
    }
    push::apply_all(server, plans, resend);
}
