//! Taking over other permission sources (PumboBridge spec §5.3): the
//! proxy's table through PumboBridge (`proxy`), plugins named in
//! `import.plugins`, or else the attachments other plugins set
//! (`attachments`). A local PumboPerms has precedence over the bridge: after
//! the first pass it asks the bridge to release the attachments and writes
//! them alone.
//!
//! When the proxy's PumboPerms rules the network (PumboBridge's `perms-mode`
//! says `proxy-rules` for the export of its current session), this plugin
//! steps back instead: it hands its attachments to PumboBridge
//! (`take-permissions`), which writes the proxy's decisions, and takes over
//! again when the proxy has no PumboPerms any more (PumboPerms spec §16). An
//! export from disk or from before a reconnect decides nothing; while a
//! connected bridge has none of this session, the first pass waits
//! ([`Rt::hold`]).
//!
//! What each source added is kept in the database (its ledger), so a later
//! import changes only that and `/pp import undo` takes it back.
//! Every call to another plugin happens outside the state (`ipc` may run
//! that plugin's handler inside it).

use pumbo_common::clock::now_ms;
use pumbo_common::text::Args;
use pumbo_perms_core::command::{self, Outcome, SourceMode, Who};
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::{merge, ui};

use super::api::{self, Server};
use super::push;
use super::state::{self, Answer, Bridge, Rt, Step};

/// The bridge plugin on Pumpkin.
pub const BRIDGE: &str = "pumbobridge";

/// The answer of another plugin, `None` when it is not there or failed.
pub fn send(plugin: &str, msg: &[u8]) -> Option<Vec<u8>> {
    crate::papi::ipc::send_ipc_message(plugin, msg).ok()?.ok()
}

fn ask(plugin: &str) -> Answer {
    state::export_answer(send(plugin, br#"{"op":"export-permissions"}"#).as_deref()).1
}

/// PumboBridge's `perms-mode`, or what an older one exports.
fn bridge() -> Bridge {
    let Some(reply) = send(BRIDGE, br#"{"op":"perms-mode"}"#) else { return Bridge::Absent };
    match state::perms_mode(&reply) {
        Some(m) => Bridge::Mode(m),
        None => {
            let (flag, answer) = state::export_answer(send(BRIDGE, br#"{"op":"export-permissions"}"#).as_deref());
            Bridge::Old(flag, answer)
        }
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
        fixed: Vec::new(),
    };
    let text = serde_json::to_string(&snap).unwrap_or_default();
    Some((merge::fingerprint(&text), snap))
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
    if source == "proxy" && mode != SourceMode::Undo && state::passive() {
        // The bridge holds only the flag that the proxy rules: nothing to import.
        return Outcome { reply: ui::warn(rt.lang(), "import-proxy-rules", &Args::new()), ..Outcome::default() };
    }
    let data = if mode == SourceMode::Undo { Err(String::new()) } else { fetch(server, source) };
    command::import_source(&mut rt.engine, who, source, data, mode, now)
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
        let taken = take();
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

/// Hands what this plugin set to PumboBridge, which takes it off; whether the
/// bridge took it.
fn take() -> bool {
    let nodes = state::with(Rt::hand_over).unwrap_or_default();
    let msg = serde_json::json!({"op": "take-permissions", "nodes": nodes}).to_string();
    send(BRIDGE, msg.as_bytes()).is_some_and(|r| {
        serde_json::from_slice::<serde_json::Value>(&r).ok().and_then(|v| v.get("ok")?.as_bool()) == Some(true)
    })
}

/// Every [`state::PASS_MS`] from the first tick (every run while the first
/// pass waits for the proxy): whether the proxy's PumboPerms rules, the first
/// import pass, then (with `import.follow`) new versions of the sources.
pub fn tick(server: &Server) {
    let now = now_ms();
    let due = state::with(|rt| {
        rt.refresh_lock(now);
        let dir = rt.dir.clone();
        let plans = rt.retry_database(now, |e| super::open_database(&dir, e.cfg.clone(), e.lang.clone(), now));
        (plans, rt.resend(), rt.pass_due(now))
    });
    state::flush_logs();
    let Some((plans, resend, due)) = due else { return };
    push::apply_plans(server, None, plans, resend);
    push::resend_due(server);
    if !due {
        return;
    }
    let bridge = bridge();
    // The mode in this answer is from before a switch below.
    let was_passive = state::passive();
    if let Some(on) = bridge.proxy_rules() {
        proxy_rules(on);
    }
    if let Bridge::Mode(m) = &bridge
        && m.mode == "local"
        && was_passive
        && state::passive()
        && take()
    {
        api::info("PumboPerms: PumboBridge had stopped writing the permissions; it writes them again");
    }
    let Some((cfg, attachments_imported, ready, step, ledger)) = state::with(|rt| {
        let follow = rt.cfg.import.follow;
        let step = if rt.hold(&bridge, now) {
            Step::Wait
        } else {
            state::decide_import(state::passive(), rt.ready, follow, rt.waiting_until, now)
        };
        let ledger = |s: &str| rt.engine.ledger(s).unwrap_or_default();
        let attachments_imported = ledger("attachments").at != 0;
        (rt.cfg.import.clone(), attachments_imported, rt.ready, step, ledger("proxy").fingerprint)
    }) else {
        return;
    };
    state::flush_logs();
    match step {
        Step::Wait => return,
        Step::GiveUp => {
            state::with(|rt| {
                rt.waiting_until = None;
                rt.warn("PumboPerms: the proxy's permission table did not come within 2 minutes; run /pp import run proxy once the proxy is connected".into());
            });
            state::flush_logs();
            return;
        }
        Step::Ask => {}
    }
    let mut answers = Vec::new();
    if cfg.enabled {
        let sources = std::iter::once("proxy".to_string()).chain(cfg.plugins.iter().cloned());
        for source in sources {
            let answer = match bridge.proxy_source(&ledger).filter(|_| source == "proxy") {
                Some(a) => a,
                None => ask(plugin_of(&source)),
            };
            answers.push((source, answer));
        }
    }
    let pending = answers.iter().any(|(_, a)| matches!(a, Answer::Pending));
    let any_data = answers.iter().any(|(_, a)| matches!(a, Answer::Data(..)));
    let fallback = if cfg.enabled && !ready && !any_data && !pending && !attachments_imported {
        attachments(server)
    } else {
        None
    };
    let result = state::with(|rt| rt.import_pass(answers, fallback, now));
    state::flush_logs();
    let Some((first, plans, resend)) = result else { return };
    if first {
        // The bridge (if any) takes its attachments back before ours go on.
        if send(BRIDGE, br#"{"op":"release-permissions"}"#).is_some() {
            api::info("PumboPerms: took over the permissions from PumboBridge");
        }
    }
    push::apply_plans(server, None, plans, resend);
}
