//! Export and import of the whole database as JSON.
//!
//! ```json
//! { "format": "pumboperms", "version": 1, "exported": 1760000000000,
//!   "groups": [ { "name": "vip", "weight": 10, "data": { "permissions": [ ... ] } } ],
//!   "users": [ { "uuid": "...", "name": "Steve", "data": { "parents": [ ... ] } } ],
//!   "tracks": [ { "name": "ranks", "groups": ["default", "vip"] } ] }
//! ```
//!
//! An import checks every name and node first and replaces everything at once.
//! Entries no command could have written are fixed or left out, each with a
//! line in [`Snapshot::fixed`]. A group or track with an invalid name comes in
//! under a valid form of it, and every reference follows. A group with nothing
//! usable in its name is left out, and so is every holder that inherits it
//! (never a holder without one of its parents); a track comes with every rung
//! or not at all. Players and UUIDs with invalid names, nodes that are not
//! nodes, parents without their group, `group.<name>` set as a permission,
//! texts that are too long or have control characters and meta keys that are
//! empty or have spaces are left out; user names that are not Minecraft
//! names, contexts in another spelling and duplicates are fixed. Only a file
//! that is not an export at all is refused.

use std::collections::{HashMap, HashSet};

use serde::ser::{Error as _, SerializeSeq, SerializeStruct};
use serde::{Deserialize, Serialize, Serializer};

use crate::model::{Data, Group, MetaKind, Track, User, is_valid_meta_key, is_valid_text};
use crate::node;
use crate::perms::{DEFAULT_GROUP, Perms};
use crate::store::PermsStore;

pub const FORMAT: &str = "pumboperms";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub exported: u64,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub users: Vec<User>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Players a source knows only by nickname (`uuid` empty), such as
    /// PumboProx's `permissions.yml`: imported at their first join.
    #[serde(default, rename = "by-name", skip_serializing_if = "Vec::is_empty")]
    pub by_name: Vec<User>,
    /// Set by PumboPerms on PumboProx: the proxy's PumboPerms rules the
    /// network, a PumboPerms on a server behind it steps back.
    #[serde(default, rename = "proxy-rules", skip_serializing_if = "std::ops::Not::not")]
    pub proxy_rules: bool,
    /// What [`parse`] fixed or left out, one line each, for the import reply.
    #[serde(skip)]
    pub fixed: Vec<String>,
}

/// The database with the players in memory (users with data only, sorted).
/// Players kept on disk are not in it: an export goes through [`write`].
pub fn snapshot(perms: &Perms, now: u64) -> Snapshot {
    let mut users: Vec<User> = perms.users().filter(|u| !u.data.is_empty()).cloned().collect();
    users.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    Snapshot {
        format: FORMAT.into(),
        version: VERSION,
        exported: now,
        groups: perms.groups().cloned().collect(),
        users,
        tracks: perms.tracks().cloned().collect(),
        by_name: Vec::new(),
        proxy_rules: false,
        fixed: Vec::new(),
    }
}

/// [`snapshot`] as pretty JSON, so also without the players kept on disk.
pub fn dump(perms: &Perms, now: u64) -> String {
    serde_json::to_string_pretty(&snapshot(perms, now)).unwrap_or_default()
}

/// The text of [`dump`], written to `w` with the players read from
/// `players` one at a time instead of held in memory. Each record goes
/// without its entries that ended by `now`; one left empty is left out.
pub fn write(w: &mut dyn std::io::Write, perms: &Perms, players: &PermsStore, now: u64) -> std::io::Result<()> {
    let all = Streamed { perms, players, now };
    all.serialize(&mut serde_json::Serializer::pretty(w)).map_err(std::io::Error::other)
}

/// A [`Snapshot`] whose users come from the database as it is written.
struct Streamed<'a> {
    perms: &'a Perms,
    players: &'a PermsStore,
    now: u64,
}

impl Serialize for Streamed<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // The fields of `Snapshot` in its order; the empty ones it leaves out stay out.
        let mut out = s.serialize_struct("Snapshot", 6)?;
        out.serialize_field("format", FORMAT)?;
        out.serialize_field("version", &VERSION)?;
        out.serialize_field("exported", &self.now)?;
        out.serialize_field("groups", &self.perms.groups().collect::<Vec<_>>())?;
        out.serialize_field("users", &Players { store: self.players, now: self.now })?;
        out.serialize_field("tracks", &self.perms.tracks().collect::<Vec<_>>())?;
        out.end()
    }
}

/// The players of the database, serialized one by one as they are read.
struct Players<'a> {
    store: &'a PermsStore,
    now: u64,
}

impl Serialize for Players<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(None)?;
        let mut failed = None;
        let read = self.store.each_user(&mut |mut user| {
            user.data.expire(self.now);
            if user.data.is_empty() {
                return true;
            }
            seq.serialize_element(&user).map_err(|e| failed = Some(e)).is_ok()
        });
        if let Some(e) = failed {
            return Err(e);
        }
        read.map_err(S::Error::custom)?;
        seq.end()
    }
}

/// Reads and checks an export. Nodes are normalized and the entries listed in
/// the module docs are fixed or left out; only a text that is not an export
/// of a known format and version is an error.
pub fn parse(text: &str) -> Result<Snapshot, String> {
    let snap: Snapshot = serde_json::from_str(text).map_err(|e| not_an_export(&e))?;
    check(snap)
}

/// Why a text is not an export. serde quotes parts of the text as they are,
/// so its line breaks and other control characters come escaped.
pub fn not_an_export(e: &serde_json::Error) -> String {
    format!("not a PumboPerms export: {}", e.to_string().escape_debug())
}

/// The checks of [`parse`] on an export already read (for one embedded in a
/// larger message).
pub fn check(mut snap: Snapshot) -> Result<Snapshot, String> {
    if snap.format != FORMAT {
        return Err(format!("unknown format {:?}", snap.format));
    }
    if snap.version > VERSION {
        return Err(format!("export version {} is newer than this version understands ({VERSION})", snap.version));
    }
    let mut fixed = Vec::new();
    // Names go into lines with {:?}: a name with a line break cannot fake a log line.
    let group_names = renames(snap.groups.iter().map(|g| g.name.as_str()), "group", &mut fixed);
    let track_names = renames(snap.tracks.iter().map(|t| t.name.as_str()), "track", &mut fixed);
    let rename = |names: &HashMap<String, Option<String>>, name: &mut String| {
        if let Some(Some(new)) = names.get(name.as_str()) {
            name.clone_from(new);
        }
    };
    for g in &mut snap.groups {
        rename(&group_names, &mut g.name);
        g.data.parents.iter_mut().for_each(|p| rename(&group_names, &mut p.group));
    }
    for u in snap.users.iter_mut().chain(&mut snap.by_name) {
        u.data.parents.iter_mut().for_each(|p| rename(&group_names, &mut p.group));
    }
    for t in &mut snap.tracks {
        rename(&track_names, &mut t.name);
        t.groups.iter_mut().for_each(|g| rename(&group_names, g));
    }
    // A group without a usable name kept the old one, which marks what needs it.
    let mut gone: HashSet<String> =
        group_names.into_iter().filter(|(_, new)| new.is_none()).map(|(old, _)| old).collect();
    snap.groups.retain(|g| !gone.contains(&g.name));
    snap.tracks.retain(|t| track_names.get(&t.name) != Some(&None));
    loop {
        let mut more = Vec::new();
        snap.groups.retain(|g| {
            let keep = keeps_parents(&gone, &format!("group {}", g.name), &g.data, &mut fixed);
            if !keep {
                more.push(g.name.clone());
            }
            keep
        });
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    let mut left_out = |ok: bool, what: &str, name: &str, why: &str| {
        if !ok {
            fixed.push(format!("{what} {name:?} left out, {why}"));
        }
        ok
    };
    snap.users.retain_mut(|u| match pumbo_common::id::Uuid::parse(&u.uuid) {
        Some(id) => {
            u.uuid = id.to_string();
            true
        }
        None => left_out(false, "user", &u.uuid, "not a UUID"),
    });
    snap.by_name
        .retain(|u| left_out(pumbo_common::id::is_java_name(&u.name), "player", &u.name, "not a Minecraft name"));
    snap.users.retain(|u| keeps_parents(&gone, &format!("user {}", u.uuid), &u.data, &mut fixed));
    snap.by_name.retain(|u| keeps_parents(&gone, &format!("player {}", u.name), &u.data, &mut fixed));
    let mut groups: HashSet<String> = snap.groups.iter().map(|g| g.name.clone()).collect();
    groups.insert(DEFAULT_GROUP.into());
    // A track without one of its rungs would promote past it.
    snap.tracks.retain(|t| {
        let Some(g) = t.groups.iter().find(|g| !groups.contains(*g)) else { return true };
        let why = if gone.contains(g) { "is left out" } else { "is not in the import" };
        fixed.push(format!("track {} left out, its group {g:?} {why}", t.name));
        false
    });
    for g in &mut snap.groups {
        let who = format!("group {}", g.name);
        if !is_valid_text(&g.display_name) {
            fixed.push(format!("{who}: display name left out ({TEXT_RULE})"));
            g.display_name.clear();
        }
        check_data(&mut g.data, &groups, &who, &mut fixed);
    }
    for u in &mut snap.users {
        let who = format!("user {}", u.uuid);
        if !u.name.is_empty() && !pumbo_common::id::is_java_name(&u.name) {
            fixed.push(format!("{who}: name {:?} is not a Minecraft name, shown by UUID", u.name));
            u.name.clear();
        }
        check_data(&mut u.data, &groups, &who, &mut fixed);
    }
    for u in &mut snap.by_name {
        u.uuid.clear();
        let who = format!("player {}", u.name);
        check_data(&mut u.data, &groups, &who, &mut fixed);
    }
    snap.fixed = fixed;
    Ok(snap)
}

/// New names for the invalid ones among `names`: [`valid_name`], or `None`
/// when nothing of the name is usable, each with a line. Valid names keep
/// theirs and come first, so a file maps the same way at every import.
fn renames<'a>(
    names: impl Iterator<Item = &'a str> + Clone,
    what: &str,
    fixed: &mut Vec<String>,
) -> HashMap<String, Option<String>> {
    let mut taken: HashSet<String> = names.clone().filter(|n| node::is_valid_name(n)).map(str::to_string).collect();
    let mut out = HashMap::new();
    for name in names {
        if node::is_valid_name(name) || out.contains_key(name) {
            continue;
        }
        let new = valid_name(name, &taken);
        match &new {
            Some(n) => {
                fixed.push(format!("{what} {name:?} imported as {n}"));
                taken.insert(n.clone());
            }
            None => fixed.push(format!("{what} {name:?} left out, no valid {what} name can be made of it")),
        }
        out.insert(name.to_string(), new);
    }
    out
}

/// A valid group or track name made of `name`: lower case, Polish letters
/// without their marks, `-` for every other character outside `a-z 0-9 _`
/// (never two in a row or at either end), cut to the length limit, then
/// `-2`, `-3`... while `taken` has it. `None` when nothing is left.
fn valid_name(name: &str, taken: &HashSet<String>) -> Option<String> {
    let mut base = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            'a'..='z' | '0'..='9' | '_' => c,
            _ => '-',
        };
        if c != '-' || !base.ends_with('-') {
            base.push(c);
        }
    }
    let base = base.trim_matches('-');
    if base.is_empty() {
        return None;
    }
    let fit = |suffix: String| {
        let room = node::MAX_NAME_LEN.saturating_sub(suffix.len());
        format!("{}{suffix}", base.get(..room).unwrap_or(base).trim_end_matches('-'))
    };
    std::iter::once(String::new()).chain((2u32..).map(|n| format!("-{n}"))).map(fit).find(|n| !taken.contains(n))
}

/// Whether a holder stays: one that inherits a group in `gone` goes whole,
/// with a line.
fn keeps_parents(gone: &HashSet<String>, who: &str, data: &Data, fixed: &mut Vec<String>) -> bool {
    match data.parents.iter().find(|p| gone.contains(&p.group)) {
        Some(p) => {
            fixed.push(format!("{who} left out, it inherits group {:?}, which is left out", p.group));
            false
        }
        None => true,
    }
}

const TEXT_RULE: &str = "over 256 bytes or with control characters";

/// Checks one holder's data: fixes and leaves out what the module docs list.
fn check_data(data: &mut Data, groups: &HashSet<String>, who: &str, fixed: &mut Vec<String>) {
    let mut clean = Data::default();
    for mut p in std::mem::take(&mut data.permissions) {
        p.node = match node::normalize(&p.node) {
            Ok(n) => n,
            Err(e) => {
                fixed.push(format!("{who}: {:?} left out, {e}", p.node));
                continue;
            }
        };
        if let Some(group) = p.node.strip_prefix(node::GROUP_PREFIX) {
            fixed.push(format!("{who}: {} left out, membership is given with parent add {group}", p.node));
            continue;
        }
        match p.context.canonical() {
            Ok(c) => p.context = c,
            Err(e) => {
                fixed.push(format!("{who}: {} left out, {e}", p.node));
                continue;
            }
        }
        let node = p.node.clone();
        if clean.set_permission(p).is_some() {
            fixed.push(format!("{who}: {node} given twice, the last one kept"));
        }
    }
    for mut p in std::mem::take(&mut data.parents) {
        // An invalid group name is never among the groups of the import.
        if !groups.contains(&p.group) {
            fixed.push(format!("{who}: parent {:?} left out, the import has no such group", p.group));
            continue;
        }
        match p.context.canonical() {
            Ok(c) => p.context = c,
            Err(e) => {
                fixed.push(format!("{who}: parent {} left out, {e}", p.group));
                continue;
            }
        }
        let group = p.group.clone();
        if !clean.add_parent(p) {
            fixed.push(format!("{who}: parent {group} given twice"));
        }
    }
    for mut m in std::mem::take(&mut data.meta) {
        let what = match m.kind {
            MetaKind::Prefix => "a prefix".to_string(),
            MetaKind::Suffix => "a suffix".to_string(),
            MetaKind::Meta => format!("meta {:?}", m.key),
        };
        if !is_valid_text(&m.value) {
            fixed.push(format!("{who}: {what} left out ({TEXT_RULE})"));
            continue;
        }
        if m.kind == MetaKind::Meta && !is_valid_meta_key(&m.key) {
            fixed.push(format!("{who}: {what} left out (key empty, with spaces, {TEXT_RULE})"));
            continue;
        }
        match m.context.canonical() {
            Ok(c) => m.context = c,
            Err(e) => {
                fixed.push(format!("{who}: {what} left out, {e}"));
                continue;
            }
        }
        clean.meta.push(m);
    }
    *data = clean;
}

/// A file name for an export: `[a-z0-9_-]`, at most 64 characters.
pub fn is_valid_file_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// `pumboperms-2026-10-08-153000` (UTC) for an export without a name.
#[expect(clippy::integer_division, reason = "whole units of time")]
pub fn default_file_name(now_ms: u64) -> String {
    let secs = now_ms / 1000;
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!("pumboperms-{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Days since 1970-01-01 to a date (proleptic Gregorian).
#[expect(clippy::integer_division, reason = "the calendar algorithm divides in whole units")]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, u32::try_from(m).unwrap_or(1), u32::try_from(d).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Contexts, Query};
    use crate::model::{ParentNode, PermNode};

    const U: &str = "00000000-0000-0000-0000-000000000001";

    #[test]
    fn round_trip() {
        let mut p = Perms::new();
        p.create_group("vip").unwrap();
        p.edit_group("vip", |g| {
            g.weight = 10;
            g.data.set_permission(PermNode {
                node: "fly".into(),
                value: true,
                context: Contexts::global(),
                expiry: None,
            });
        })
        .unwrap();
        p.edit_user(U, "Steve", |d| {
            d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None })
        });
        p.create_track("ranks").unwrap();
        p.track_insert("ranks", "vip", None).unwrap();
        let json = dump(&p, 5);
        let snap = parse(&json).unwrap();
        let (mut q, _) = Perms::from_parts(snap.groups, snap.users, snap.tracks);
        assert_eq!(q.check(U, "fly", &Query::default(), 0), Some(true));
        assert_eq!(q.track("ranks").unwrap().groups, vec!["vip"]);
    }

    /// Saves everything `p` holds and reads it back as an export of the database.
    fn streamed(p: &Perms, now: u64) -> String {
        let store = PermsStore::in_memory().unwrap();
        let mut d = crate::perms::Dirty::default();
        d.groups.extend(p.groups().map(|g| g.name.clone()));
        d.users.extend(p.users().map(|u| u.uuid.clone()));
        d.tracks.extend(p.tracks().map(|t| t.name.clone()));
        store.commit(p, &d, None, 0).unwrap();
        let mut out = Vec::new();
        write(&mut out, p, &store, now).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn streamed_export_equals_the_old_text() {
        let mut p = Perms::new();
        assert_eq!(streamed(&p, 7), dump(&p, 7), "nothing but the default group");
        p.create_group("vip").unwrap();
        p.edit_group("vip", |g| {
            g.weight = 10;
            g.display_name = "VIP".into();
            g.data.add_affix(MetaKind::Prefix, 5, "&6[VIP] ", &Contexts::global(), None, false);
        })
        .unwrap();
        for (uuid, name) in [(U, "Steve"), ("00000000-0000-0000-0000-000000000002", "Alex \"quoted\"")] {
            p.edit_user(uuid, name, |d| {
                d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: Some(99_000) });
                d.set_permission(PermNode {
                    node: "fly".into(),
                    value: false,
                    context: Contexts::parse(&["server=lobby".to_string()]).unwrap(),
                    expiry: None,
                });
            });
        }
        p.edit_user("00000000-0000-0000-0000-000000000003", "Empty", |_| ());
        p.create_track("ranks").unwrap();
        p.track_insert("ranks", "vip", None).unwrap();
        assert_eq!(streamed(&p, 1_000), dump(&p, 1_000));
    }

    #[test]
    fn export_holds_every_player() {
        let lang = pumbo_common::lang::Lang::load(&[pumbo_common::lang::COMMON], "en", None).0;
        let mut e = crate::engine::Engine::lazy(crate::config::CoreCfg::default(), lang);
        let users: Vec<User> = (0..300u32)
            .map(|i| User {
                uuid: format!("00000000-0000-4000-8000-{i:012}"),
                name: format!("P{i}"),
                data: Data { permissions: vec![perm_of(&format!("n.{i}"))], ..Data::default() },
            })
            .collect();
        let (all, mut d) = Perms::from_parts(Vec::new(), users, Vec::new());
        d.users.extend(all.users().map(|u| u.uuid.clone()));
        e.store().unwrap().commit(&all, &d, None, 0).unwrap();
        e.reload_data();
        assert!(e.perms.users().next().is_none(), "nobody in memory");
        let dir = std::env::temp_dir().join(format!("pumbo-perms-export-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = crate::files::export_to(&e, &dir, "all", 5).unwrap();
        let snap = parse(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
        assert_eq!(snap.users, snapshot(&all, 5).users);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_leaves_out_expired_entries() {
        let mut p = Perms::new();
        p.create_group("vip").unwrap();
        p.edit_user(U, "Steve", |d| {
            d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: Some(100) });
            d.set_permission(perm_of("fly"));
        });
        p.edit_user("00000000-0000-0000-0000-000000000002", "Alex", |d| {
            d.add_parent(ParentNode { group: "vip".into(), context: Contexts::global(), expiry: Some(100) })
        });
        let snap = parse(&streamed(&p, 100)).unwrap();
        assert_eq!(snap.users.len(), 1, "one with nothing left is left out");
        assert_eq!((snap.users[0].data.parents.len(), snap.users[0].data.permissions.len()), (0, 1));
        assert_eq!(parse(&streamed(&p, 99)).unwrap().users.len(), 2, "not ended yet");
    }

    fn perm_of(node: &str) -> PermNode {
        PermNode { node: node.into(), value: true, context: Contexts::global(), expiry: None }
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("nope").is_err());
        assert!(parse(r#"{"format":"otherperms","version":1}"#).is_err());
        assert!(parse(r#"{"format":"pumboperms","version":99}"#).is_err());
        let ok = parse(r#"{"format":"pumboperms","version":1,"groups":[{"name":"a","data":{"permissions":[{"node":"PumboBans:Ban","value":true}]}}]}"#)
            .unwrap();
        assert_eq!(ok.groups[0].data.permissions[0].node, "pumbo.bans.ban");
    }

    fn names<'a>(items: impl IntoIterator<Item = &'a String>) -> Vec<&'a str> {
        items.into_iter().map(String::as_str).collect()
    }

    /// Invalid group and track names come in under valid ones, and every
    /// reference follows: a track keeps all its rungs, a holder its parents.
    #[test]
    fn invalid_names_come_in_under_valid_ones() {
        const BAD_GROUP: &str = "zła grupa!";
        const BAD_PLAYER: &str = "ŻółwiaGęśŚliczna_bardzo_długa_nazwa_z_ogonkami";
        let (upper, lower) = ("A".repeat(40), "a".repeat(40));
        let json = serde_json::json!({
            "format": "pumboperms", "version": 1,
            "groups": [
                {"name": BAD_GROUP, "data": {"permissions": [{"node": "fly", "value": false}]}},
                {"name": "Zła Grupa!"},
                {"name": "Początkujący"},
                {"name": "vip", "data": {
                    "permissions": [{"node": "fly", "value": true}, {"node": "a b", "value": true}],
                    "parents": [{"group": BAD_GROUP}]}},
                {"name": upper},
                {"name": lower},
            ],
            "users": [
                {"uuid": "x", "name": "Steve"},
                {"uuid": U, "name": "Steve", "data": {"parents": [{"group": "vip"}, {"group": BAD_GROUP}]}},
            ],
            "by-name": [
                {"name": BAD_PLAYER, "data": {"parents": [{"group": "vip"}]}},
                {"name": "Jeb_", "data": {"parents": [{"group": "Początkujący"}]}},
            ],
            "tracks": [
                {"name": "Rangi Główne", "groups": ["default", "Początkujący", "vip"]},
                {"name": "ranks", "groups": ["default", BAD_GROUP, "vip"]},
            ],
        });
        let snap = parse(&json.to_string()).unwrap();
        let (a36, a34) = ("a".repeat(36), format!("{}-2", "a".repeat(34)));
        let groups: Vec<&str> = snap.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(groups, ["zla-grupa", "zla-grupa-2", "poczatkujacy", "vip", &a36, &a34]);
        assert!(!snap.groups[0].data.permissions[0].value, "the deny comes along");
        let parents = |d: &Data| d.parents.iter().map(|p| p.group.clone()).collect::<Vec<_>>();
        assert_eq!(parents(&snap.groups[3].data), ["zla-grupa"]);
        assert!(snap.users.len() == 1 && parents(&snap.users[0].data) == ["vip", "zla-grupa"]);
        assert!(snap.by_name.len() == 1 && parents(&snap.by_name[0].data) == ["poczatkujacy"]);
        let tracks: Vec<(&str, Vec<&str>)> = snap.tracks.iter().map(|t| (t.name.as_str(), names(&t.groups))).collect();
        assert_eq!(
            tracks,
            [("rangi-glowne", vec!["default", "poczatkujacy", "vip"]), ("ranks", vec!["default", "zla-grupa", "vip"])]
        );
        // Every bad name is in its line whole, the long one too.
        for line in [
            format!("group {BAD_GROUP:?} imported as zla-grupa"),
            "group \"Zła Grupa!\" imported as zla-grupa-2".to_string(),
            "group \"Początkujący\" imported as poczatkujacy".to_string(),
            format!("group {upper:?} imported as {a36}"),
            format!("group {lower:?} imported as {a34}"),
            "track \"Rangi Główne\" imported as rangi-glowne".to_string(),
            format!("player {BAD_PLAYER:?} left out, not a Minecraft name"),
            "user \"x\" left out, not a UUID".to_string(),
        ] {
            assert!(snap.fixed.contains(&line), "{line} not in {:?}", snap.fixed);
        }
        assert!(snap.fixed.iter().any(|f| f.starts_with("group vip: \"a b\" left out")), "{:?}", snap.fixed);
        assert_eq!(snap.fixed.len(), 9, "{:?}", snap.fixed);
        let again = parse(&json.to_string()).unwrap();
        assert_eq!((again.groups, again.tracks), (snap.groups, snap.tracks), "the same file maps the same way");
    }

    /// A name with nothing usable leaves out its entry and everything that
    /// needs it, down the inheritance: no track loses a rung, no holder a parent.
    #[test]
    fn nothing_usable_leaves_out_what_needs_it() {
        const U2: &str = "00000000-0000-0000-0000-000000000002";
        let json = serde_json::json!({
            "format": "pumboperms", "version": 1,
            "groups": [
                {"name": "日本", "data": {"permissions": [{"node": "fly", "value": false}]}},
                {"name": "child", "data": {"parents": [{"group": "日本"}]}},
                {"name": "grandchild", "data": {"parents": [{"group": "child"}]}},
                {"name": "vip"},
                {"name": ""},
            ],
            "users": [
                {"uuid": U, "data": {"parents": [{"group": "vip"}, {"group": "grandchild"}]}},
                {"uuid": U2, "data": {"parents": [{"group": "vip"}]}},
            ],
            "by-name": [{"name": "Jeb_", "data": {"parents": [{"group": "日本"}]}}],
            "tracks": [
                {"name": "path", "groups": ["default", "日本"]},
                {"name": "up", "groups": ["default", "vip", "child"]},
                {"name": "ghosts", "groups": ["default", "ghost"]},
                {"name": "ok", "groups": ["default", "vip"]},
                {"name": "!!!", "groups": ["vip"]},
            ],
        });
        let snap = parse(&json.to_string()).unwrap();
        assert_eq!(snap.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["vip"]);
        assert!(snap.users.len() == 1 && snap.users[0].uuid == U2);
        assert!(snap.by_name.is_empty());
        assert_eq!(snap.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["ok"]);
        let lines = [
            "group \"日本\" left out, no valid group name can be made of it".to_string(),
            "group \"\" left out, no valid group name can be made of it".to_string(),
            "track \"!!!\" left out, no valid track name can be made of it".to_string(),
            "group child left out, it inherits group \"日本\", which is left out".to_string(),
            "group grandchild left out, it inherits group \"child\", which is left out".to_string(),
            format!("user {U} left out, it inherits group \"grandchild\", which is left out"),
            "player Jeb_ left out, it inherits group \"日本\", which is left out".to_string(),
            "track path left out, its group \"日本\" is left out".to_string(),
            "track up left out, its group \"child\" is left out".to_string(),
            "track ghosts left out, its group \"ghost\" is not in the import".to_string(),
        ];
        for line in &lines {
            assert!(snap.fixed.contains(line), "{line} not in {:?}", snap.fixed);
        }
        assert_eq!(snap.fixed.len(), lines.len(), "{:?}", snap.fixed);
    }

    /// Texts from the file reach the lines escaped, serde's messages too.
    #[test]
    fn fix_lines_have_no_line_breaks() {
        let json = serde_json::json!({
            "format": "pumboperms", "version": 1,
            "groups": [{"name": "vip", "data": {"permissions": [
                {"node": "a\nb", "value": true},
                {"node": "fly", "value": true, "context": {"wor\nld": "x"}},
                {"node": "build", "value": true, "context": {"world": "a\nb"}},
            ]}}],
        });
        let snap = parse(&json.to_string()).unwrap();
        assert_eq!(snap.fixed.len(), 3, "{:?}", snap.fixed);
        assert!(snap.fixed.iter().all(|f| !f.contains('\n')), "{:?}", snap.fixed);
        for part in [r"character '\n'", r#"unknown context "wor\nld""#, r#"invalid context value "a\nb""#] {
            assert!(snap.fixed.iter().any(|f| f.contains(part)), "{part} not in {:?}", snap.fixed);
        }
        let bad = r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip","data":{"meta":[{"kind":"pre\nfix","value":"x"}]}}]}"#;
        let err = parse(bad).unwrap_err();
        assert!(!err.contains('\n') && err.contains(r"pre\nfix"), "{err}");
        let err = parse("{\"format\":\"a\\nb\",\"version\":1}").unwrap_err();
        assert!(!err.contains('\n'), "{err}");
    }

    #[test]
    fn file_names() {
        assert!(is_valid_file_name("backup-1_a"));
        assert!(!is_valid_file_name("../etc") && !is_valid_file_name("A") && !is_valid_file_name(""));
        assert_eq!(default_file_name(0), "pumboperms-1970-01-01-000000");
        assert_eq!(default_file_name(1_759_937_400_000), "pumboperms-2025-10-08-153000");
        assert_eq!(default_file_name(951_782_400_000), "pumboperms-2000-02-29-000000");
    }
}
