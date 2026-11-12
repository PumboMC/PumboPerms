//! Imports from a file or another permission source, and pending entries at a join.

use std::collections::HashSet;

use pumbo_common::text::{self, Args};

use crate::context;
use crate::engine::Engine;
use crate::export::Snapshot;
use crate::merge::{self, Ledger};
use crate::store::LogEntry;
use crate::ui::{self, Text, value};

use super::{Outcome, SourceMode, Who};

/// Runs an import after the platform read the file: replaces everything and
/// saves it (the platform exports a backup first).
pub fn import(e: &mut Engine, who: &Who, file: &str, json: &str, backup: &str, now: u64) -> Outcome {
    let mut snap = match crate::export::parse(json) {
        Ok(s) => s,
        Err(err) => return Outcome::reply(ui::error(&e.lang, "import-failed", &Args::new().arg(value(err)))),
    };
    let counts = (snap.groups.len(), snap.users.len(), snap.tracks.len());
    let fixed = std::mem::take(&mut snap.fixed);
    let worlds = world_warning(e, &snap);
    let before = e.perms.clone();
    e.perms.replace_all(snap.groups, snap.users, snap.tracks);
    let log = LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: "import".into(),
        target: file.to_string(),
        action: format!("import {file}"),
    };
    let saved = match e.store() {
        Some(store) if e.broken.is_none() => {
            store.replace_all(&e.perms, Some(&log), e.cfg.log.max_entries).map_err(|x| x.to_string())
        }
        _ => Err(e.broken.clone().unwrap_or_else(|| "no database".into())),
    };
    if let Err(err) = saved {
        e.perms = before;
        return Outcome::reply(ui::error(&e.lang, "import-failed", &Args::new().arg(value(err))));
    }
    e.changed_everything();
    let a =
        Args::new().arg(value(counts.0)).arg(value(counts.1)).arg(value(counts.2)).arg(value(file)).arg(value(backup));
    let mut reply = ui::success(&e.lang, "imported", &a);
    fixed_lines(e, &fixed, &mut reply);
    reply.extend(worlds);
    Outcome { reply, changed: true, log: Some(log), action: None }
}

/// `/pp import run|preview|undo <source>` after the platform asked the source:
/// `data` is its fingerprint and export. What the source added last time comes
/// from its ledger in the database, and the new ledger is saved together with
/// the import: entries the source dropped go, new ones are added, and the rest
/// stays as it is now (also an imported entry an admin removed).
pub fn import_source(
    e: &mut Engine,
    who: &Who,
    source: &str,
    data: Result<(String, Snapshot), String>,
    mode: SourceMode,
    now: u64,
) -> Outcome {
    let fail = |e: &Engine, why: String| {
        Outcome::reply(ui::error(&e.lang, "import-source-failed", &Args::new().arg(value(source)).arg(value(why))))
    };
    let log = |action: String| LogEntry {
        time: now,
        actor: who.name.clone(),
        actor_uuid: who.uuid.clone().unwrap_or_default(),
        kind: "import".into(),
        target: source.to_string(),
        action: cut(action, 1000),
    };
    let ledger = match e.ledger(source) {
        Ok(l) => l,
        Err(why) => return fail(e, why),
    };
    if mode == SourceMode::Undo {
        let before = e.perms.clone();
        let (dirty, n) = merge::undo(&mut e.perms, &ledger);
        let entry = log(format!("import undo {source}: {n} entries"));
        if let Err(why) = e.commit_import(&dirty, Some(&entry), source, &Ledger::default()) {
            e.perms = before;
            return fail(e, why);
        }
        let a = Args::new().arg(value(source)).arg(value(n));
        return Outcome {
            reply: ui::success(&e.lang, "import-source-undone", &a),
            changed: n > 0,
            ..Outcome::default()
        };
    }
    let (fingerprint, mut snap) = match data {
        Ok(d) => d,
        Err(why) => return fail(e, why),
    };
    if mode == SourceMode::Run && ledger.fingerprint == fingerprint {
        return Outcome::reply(ui::info(&e.lang, "import-source-same", &Args::new().arg(value(source))));
    }
    let worlds = world_warning(e, &snap);
    // Players by nickname: the UUID this source bound the name to before, or
    // one that joined with an online-mode account; the rest at such a join.
    let mut pending = Vec::new();
    let mut resolved = std::collections::BTreeMap::new();
    for u in std::mem::take(&mut snap.by_name) {
        let key = pumbo_common::id::name_key(&u.name);
        let bound = ledger.resolved.get(&key).cloned();
        let uuid = bound.or_else(|| e.find_user(&u.name).filter(|k| k.online).map(|k| k.uuid));
        match uuid {
            Some(uuid) => {
                resolved.insert(key, uuid.clone());
                snap.users.push(crate::model::User { uuid, name: u.name, data: u.data });
            }
            None => pending.push(u),
        }
    }
    let bound: Vec<String> = snap
        .users
        .iter()
        .filter(|u| resolved.get(&pumbo_common::id::name_key(&u.name)) == Some(&u.uuid))
        .map(|u| format!("{} -> {}", u.name, u.uuid))
        .collect();
    let notes = |e: &Engine, reply: &mut Text| {
        if !bound.is_empty() {
            let a = Args::new().arg(value(bound.len())).arg(value(bound.join(", ")));
            reply.extend(ui::info(&e.lang, "import-source-resolved", &a));
        }
        if !pending.is_empty() {
            let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
            let a = Args::new().arg(value(pending.len())).arg(value(names.join(", ")));
            let key = if e.cfg.names_checked { "import-source-pending" } else { "import-source-name-only" };
            reply.extend(ui::info(&e.lang, key, &a));
        }
    };
    // What the source no longer has goes; what it still has stays as it is.
    let all: HashSet<String> = merge::candidates(&snap).iter().map(merge::Entry::key).collect();
    let (kept, gone): (Vec<merge::Entry>, Vec<merge::Entry>) =
        ledger.entries.iter().cloned().partition(|x| all.contains(&x.key()));
    let before = e.perms.clone();
    let (mut dirty, removed) = merge::undo(&mut e.perms, &Ledger { entries: gone.clone(), ..Ledger::default() });
    let mut plan = merge::plan(&e.perms, &snap);
    merge::skip_recorded(&mut plan, &e.perms, &ledger.entries);
    if mode == SourceMode::Preview {
        e.perms = before;
        let a = Args::new().arg(value(source)).arg(value(plan.entries.len())).arg(value(plan.conflicts.len()));
        let mut reply = ui::header(&e.lang, "import-source-preview", &a);
        fixed_lines(e, &snap.fixed, &mut reply);
        reply.extend(worlds);
        for entry in plan.entries.iter().take(15) {
            reply.push(ui::row(1, "", &format!("{}+ {}", ui::YES, describe(entry))));
        }
        for entry in gone.iter().take(15) {
            reply.push(ui::row(1, "", &format!("{}- {}", ui::NO, describe(entry))));
        }
        for c in plan.conflicts.iter().take(10) {
            reply.push(ui::row(1, "", &format!("{}! {c}", ui::WARN)));
        }
        notes(e, &mut reply);
        return Outcome::reply(reply);
    }
    let (more, added) = merge::apply(&mut e.perms, &plan);
    dirty.merge(more);
    let mut action = format!("import {source}: {} entries", added.len());
    if removed > 0 {
        action.push_str(&format!(", {removed} taken back"));
    }
    if !plan.conflicts.is_empty() {
        action.push_str(&format!(", kept on conflict: {}", plan.conflicts.join("; ")));
    }
    if !bound.is_empty() {
        action.push_str(&format!(", by name: {}", bound.join(", ")));
    }
    if !pending.is_empty() {
        let names: Vec<&str> = pending.iter().map(|u| u.name.as_str()).collect();
        action.push_str(&format!(", waiting for the first join: {}", names.join(", ")));
    }
    let mut entries = kept;
    entries.extend(added.iter().cloned());
    let new = Ledger { fingerprint, at: now, entries, pending: pending.clone(), resolved };
    if let Err(why) = e.commit_import(&dirty, Some(&log(action)), source, &new) {
        e.perms = before;
        return fail(e, why);
    }
    let a = Args::new().arg(value(source)).arg(value(added.len())).arg(value(plan.conflicts.len()));
    let mut reply = ui::success(&e.lang, "import-source-done", &a);
    if removed > 0 {
        reply.extend(ui::info(&e.lang, "import-source-removed", &Args::new().arg(value(removed))));
    }
    fixed_lines(e, &snap.fixed, &mut reply);
    reply.extend(worlds);
    notes(e, &mut reply);
    Outcome { reply, changed: true, ..Outcome::default() }
}

/// A player joined: what every source has for its nickname
/// ([`import_pending`]); one outcome per source that has something.
pub fn on_join(e: &mut Engine, uuid: &str, name: &str, online: bool, now: u64) -> Vec<Outcome> {
    let mut waiting = Vec::new();
    let mut damaged = Vec::new();
    for (source, ledger) in e.ledgers() {
        match ledger {
            Ok(l) if l.pending.is_empty() => {}
            Ok(_) => waiting.push(source.to_string()),
            Err(why) => damaged.push((source.to_string(), why.to_string())),
        }
    }
    // A damaged ledger once, not at every join.
    damaged.retain(|(source, _)| e.reported.insert(source.clone()));
    let mut out: Vec<Outcome> = damaged
        .into_iter()
        .map(|(source, why)| {
            let a = Args::new().arg(value(&source)).arg(value(why));
            Outcome::reply(ui::error(&e.lang, "import-source-failed", &a))
        })
        .collect();
    for source in waiting {
        out.extend(import_pending(e, &source, uuid, name, online, now));
    }
    out
}

/// A player joined: what `source` has for its nickname (`ledger.pending`) is
/// added now, only adding like every import. Only for an account the host
/// checked with Mojang (`online`): anyone can take a nickname in offline mode,
/// so an offline account gets nothing and the entries keep waiting (the reply
/// says so, `changed` stays false). `None` when the source has nothing for this
/// name.
pub fn import_pending(e: &mut Engine, source: &str, uuid: &str, name: &str, online: bool, now: u64) -> Option<Outcome> {
    let mut ledger = e.ledger(source).ok()?;
    let key = pumbo_common::id::name_key(name);
    let pos = ledger.pending.iter().position(|u| pumbo_common::id::name_key(&u.name) == key)?;
    if !online {
        let a = Args::new().arg(value(source)).arg(value(name));
        let key = if e.cfg.names_checked { "import-source-offline" } else { "import-source-joined-name-only" };
        return Some(Outcome::reply(ui::warn(&e.lang, key, &a)));
    }
    let waiting = ledger.pending.remove(pos);
    let user = crate::model::User { uuid: uuid.to_string(), name: name.to_string(), data: waiting.data };
    let plan = merge::plan(&e.perms, &Snapshot { users: vec![user], ..Snapshot::default() });
    let before = e.perms.clone();
    let (dirty, added) = merge::apply(&mut e.perms, &plan);
    let log = LogEntry {
        time: now,
        actor: "PumboPerms".into(),
        actor_uuid: String::new(),
        kind: "import".into(),
        target: source.to_string(),
        action: format!("import {source}: {name} -> {uuid} joined, {} entries", added.len()),
    };
    let a = Args::new().arg(value(source)).arg(value(format!("{name} -> {uuid}"))).arg(value(added.len()));
    ledger.entries.extend(added);
    ledger.resolved.insert(key, uuid.to_string());
    if let Err(why) = e.commit_import(&dirty, Some(&log), source, &ledger) {
        e.perms = before;
        let a = Args::new().arg(value(source)).arg(value(why));
        return Some(Outcome::reply(ui::error(&e.lang, "import-source-failed", &a)));
    }
    Some(Outcome {
        reply: ui::success(&e.lang, "import-source-joined", &a),
        changed: true,
        log: Some(log),
        action: None,
    })
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn cut(mut text: String, max: usize) -> String {
    text.truncate(text.floor_char_boundary(max));
    text
}

/// Where nobody sees worlds, one warning with the number of `world=` entries
/// the import keeps.
fn world_warning(e: &Engine, snap: &Snapshot) -> Text {
    if e.cfg.worlds {
        return Text::new();
    }
    let data = snap.groups.iter().map(|g| &g.data).chain(snap.users.iter().chain(&snap.by_name).map(|u| &u.data));
    let count: usize = data
        .map(|d| {
            let contexts = d.permissions.iter().map(|p| &p.context);
            let contexts =
                contexts.chain(d.parents.iter().map(|p| &p.context)).chain(d.meta.iter().map(|m| &m.context));
            contexts.filter(|c| c.get(context::WORLD).is_some()).count()
        })
        .sum();
    if count == 0 {
        return Text::new();
    }
    ui::warn(&e.lang, "import-worlds", &Args::new().arg(value(count)))
}

/// What the import fixed or left out ([`crate::export::Snapshot::fixed`]).
fn fixed_lines(e: &Engine, fixed: &[String], reply: &mut Text) {
    if fixed.is_empty() {
        return;
    }
    reply.extend(ui::warn(&e.lang, "import-fixed", &Args::new().arg(value(fixed.len()))));
    for f in fixed.iter().take(20) {
        reply.push(ui::row(1, "", &format!("{}! {}", ui::WARN, text::escape(f))));
    }
}

fn describe(e: &merge::Entry) -> String {
    use merge::Entry as E;
    match e {
        E::Group { group } => format!("group {group}"),
        E::GroupNode { group, node } => format!("{group}: {} = {}", node.node, node.value),
        E::GroupParent { group, parent } => format!("{group} inherits {}", parent.group),
        E::GroupWeight { group, weight } => format!("{group}: weight {weight}"),
        E::GroupMeta { group, meta } => format!("{group}: {:?} {}", meta.kind, meta.value),
        E::UserNode { name, node, .. } => format!("{name}: {} = {}", node.node, node.value),
        E::UserParent { name, parent, .. } => format!("{name} in {}", parent.group),
    }
}
