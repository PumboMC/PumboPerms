//! Import passes, the answers of other plugins and PumboBridge, and when to ask again.

use pumbo_perms_core::command::{self, SourceMode, Who};
use pumbo_perms_core::export::{self, Snapshot};
use serde::Deserialize;

use super::{Plan, Rt, passive};

impl Rt {
    /// One import pass over the sources that answered (and the attachments of
    /// other plugins, when they are the fallback). Returns whether it was the
    /// first pass, the host changes and whether to resend command lists.
    pub fn import_pass(
        &mut self,
        answers: Vec<(String, Answer)>,
        fallback: Option<(String, Snapshot)>,
        now: u64,
    ) -> (bool, Vec<Plan>, bool) {
        if self.lock.is_none() && self.engine.broken.is_some() {
            // Another instance holds the database: no import and no first pass
            // (it would release PumboBridge's attachments for an empty engine).
            return (false, Vec::new(), self.resend());
        }
        let follow = self.cfg.import.follow;
        let pending = answers.iter().any(|(_, a)| matches!(a, Answer::Pending));
        let mut changed = false;
        let who = Who::console();
        let mut runs: Vec<(String, String, Snapshot)> = Vec::new();
        for (source, answer) in answers {
            match answer {
                Answer::Data(f, s) => runs.push((source, f, s)),
                Answer::Bad(e) => self.warn(format!("PumboPerms: cannot read permissions of {source}: {e}")),
                Answer::Pending | Answer::Absent => {}
            }
        }
        if let Some((f, s)) = fallback {
            runs.push(("attachments".into(), f, s));
        }
        for (source, fingerprint, snap) in runs {
            let ledger = match self.engine.ledger(&source) {
                Ok(l) => l,
                Err(why) => {
                    // An empty ledger would import everything again.
                    self.error(format!("PumboPerms: {source} is not imported automatically: {why}"));
                    continue;
                }
            };
            // Once per source unless `follow` (a ledger means it was imported).
            if ledger.at != 0 && !follow {
                continue;
            }
            let out =
                command::import_source(&mut self.engine, &who, &source, Ok((fingerprint, snap)), SourceMode::Run, now);
            if out.changed {
                changed = true;
                self.logs.push((false, out.reply.plain()));
            }
        }
        if pending && !self.ready {
            self.waiting_until = Some(now + WAIT_MS);
        } else if !pending {
            self.waiting_until = None;
        }
        let first = !self.ready;
        self.ready = true;
        let plans = if first {
            self.refresh_catalog();
            let plans = self.plan_all(now);
            if !passive() {
                self.forget_old_nodes();
            }
            plans
        } else if changed {
            self.after_change(now)
        } else {
            Vec::new()
        };
        (first, plans, self.resend())
    }

    /// Before the periodic pass asks PumboBridge: whether it is time.
    pub fn pass_due(&self, now: u64) -> bool {
        now >= self.next_pass
    }

    /// After PumboBridge answered: whether the pass stops here. Before the
    /// first pass, a bridge that connects to a proxy but has no live export
    /// yet holds everything: no import, no attachments, no release of the
    /// bridge, so Pumpkin's defaults apply instead of an old local database.
    /// The bridge is asked again at every run for [`HOLD_MS`], then every
    /// [`PASS_MS`] with one warning.
    pub fn hold(&mut self, bridge: &Bridge, now: u64) -> bool {
        let waiting = !self.ready && matches!(bridge, Bridge::Mode(m) if m.connected && !m.live);
        if !waiting {
            self.holding = None;
            self.next_pass = now.saturating_add(PASS_MS);
            return false;
        }
        let (since, warned) = *self.holding.get_or_insert((now, false));
        if now.saturating_sub(since) < HOLD_MS {
            self.next_pass = now;
        } else {
            if !warned {
                self.warn("PumboPerms: PumboBridge has no permission table from the proxy yet; waiting for it (Pumpkin's defaults apply until then)".into());
                self.holding = Some((since, true));
            }
            self.next_pass = now.saturating_add(PASS_MS);
        }
        true
    }
}

/// How long a "pending" source is asked again after the start.
const WAIT_MS: u64 = 120_000;
/// The periodic import pass runs this often.
pub const PASS_MS: u64 = 10_000;
/// How long the first pass waits for PumboBridge's live export, asking at
/// every run, before it warns and asks every [`PASS_MS`].
pub const HOLD_MS: u64 = 10_000;

/// One source's answer to `{"op":"export-permissions"}`.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Data(String, Snapshot),
    Pending,
    /// No such plugin (or it does not export).
    Absent,
    Bad(String),
}

/// Reads an answer to `{"op":"export-permissions"}` in one pass; also the
/// `proxy-rules` flag, which a newer export version than this one
/// understands still carries.
pub fn export_answer(reply: Option<&[u8]>) -> (bool, Answer) {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Reply {
        ok: bool,
        pending: bool,
        fingerprint: String,
        data: Option<Snapshot>,
    }
    let Some(bytes) = reply else { return (false, Answer::Absent) };
    match serde_json::from_slice::<Reply>(bytes) {
        Err(e) => (false, Answer::Bad(export::not_an_export(&e))),
        Ok(r) if !r.ok => (false, if r.pending { Answer::Pending } else { Answer::Absent }),
        Ok(Reply { data: None, .. }) => (false, Answer::Bad("no data".into())),
        Ok(Reply { fingerprint, data: Some(snap), .. }) => {
            let flag = snap.proxy_rules;
            (flag, export::check(snap).map_or_else(Answer::Bad, |s| Answer::Data(fingerprint, s)))
        }
    }
}

/// PumboBridge's answer to `{"op":"perms-mode"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BridgeMode {
    pub proxy_rules: bool,
    pub fingerprint: String,
    /// `bridge` or `local` (a local PumboPerms writes the attachments).
    pub mode: String,
    /// The export came from the proxy in the bridge's current session.
    pub live: bool,
    /// The bridge connects to a proxy.
    pub connected: bool,
}

/// PumboBridge as the periodic pass sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Bridge {
    Absent,
    /// A PumboBridge without `perms-mode`: the flag and the answer of
    /// `export-permissions`, as before.
    Old(bool, Answer),
    Mode(BridgeMode),
}

/// Reads the answer to `{"op":"perms-mode"}`; `None` when it is not one (an
/// older PumboBridge answers "unknown op").
pub fn perms_mode(reply: &[u8]) -> Option<BridgeMode> {
    let v: serde_json::Value = serde_json::from_slice(reply).ok()?;
    if v.get("ok")?.as_bool() != Some(true) {
        return None;
    }
    serde_json::from_value(v).ok()
}

impl Bridge {
    /// Whether the proxy's PumboPerms rules, when the answer can tell: only
    /// an export from the bridge's current session counts, never one from
    /// disk or from before a reconnect.
    pub fn proxy_rules(&self) -> Option<bool> {
        match self {
            Bridge::Mode(m) if m.live => Some(m.proxy_rules),
            Bridge::Old(flag, Answer::Data(..)) => Some(*flag),
            _ => None,
        }
    }

    /// The answer of the `proxy` source without asking for the export, or
    /// `None` when it has to be asked: a live export whose fingerprint is not
    /// the one imported last (`ledger_fingerprint`).
    pub fn proxy_source(&self, ledger_fingerprint: &str) -> Option<Answer> {
        match self {
            Bridge::Absent => Some(Answer::Absent),
            Bridge::Old(_, a) => Some(a.clone()),
            Bridge::Mode(m) if !m.live => Some(if m.connected { Answer::Pending } else { Answer::Absent }),
            Bridge::Mode(m) if m.fingerprint == ledger_fingerprint => Some(Answer::Absent),
            Bridge::Mode(_) => None,
        }
    }
}

/// What the periodic import pass does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing this time.
    Wait,
    /// The proxy's table did not come in time: stop waiting and say so.
    GiveUp,
    /// Ask the sources and import what they send.
    Ask,
}

/// The decision before asking: only the first pass, a source still pending
/// after the start, or every pass with `import.follow`; nothing while the
/// proxy's PumboPerms rules.
pub fn decide_import(passive: bool, ready: bool, follow: bool, waiting_until: Option<u64>, now: u64) -> Step {
    if passive {
        return Step::Wait;
    }
    if ready && !follow {
        return match waiting_until {
            Some(t) if now < t => Step::Ask,
            Some(_) => Step::GiveUp,
            None => Step::Wait,
        };
    }
    Step::Ask
}
