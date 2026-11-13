//! Changes, rights, the log and errors of `/pp`.

use super::*;

#[test]
fn messages_are_complete_in_every_language() {
    assert_eq!(check_bundle(&LANG), Vec::<String>::new());
    let (pl, w) = Lang::load(&[COMMON, LANG], "pl", None);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(pl.get("help-section"), "Uprawnienia");
}

#[test]
fn every_message_key_used_exists() {
    let (l, _) = Lang::load(&[COMMON, LANG], "en", None);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/command");
    let mut src = String::new();
    for file in std::fs::read_dir(dir).unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_some_and(|x| x == "rs") {
            src.push_str(&std::fs::read_to_string(path).unwrap());
        }
    }
    let mut missing = Vec::new();
    for (i, _) in src.match_indices('"') {
        let rest = &src[i + 1..];
        let Some(end) = rest.find('"') else { continue };
        let word = &rest[..end];
        let looks_like_key = word.contains('-')
            && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            && [
                "perm-", "parent-", "meta-", "affix-", "group-", "track-", "bad-", "desc-", "label-", "title-",
                "hover-", "limit-",
            ]
            .iter()
            .any(|p| word.starts_with(p));
        if looks_like_key && !l.has(word) {
            missing.push(word.to_string());
        }
    }
    missing.sort();
    missing.dedup();
    // keys built at run time: kind-prefix, desc-*-details are optional
    missing.retain(|k| k != "group-" && k != "track-");
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn quoting() {
    assert_eq!(split_args("a  \"b c\" d"), vec!["a", "b c", "d"]);
    assert_eq!(split_args("meta setprefix 10 \"&a[VIP] \""), vec!["meta", "setprefix", "10", "&a[VIP] "]);
    assert_eq!(split_args("x \"\" y"), vec!["x", "", "y"]);
    assert_eq!(split_args("\"say \\\"hi\\\"\""), vec!["say \"hi\""]);
    assert!(split_args("   ").is_empty());
}

#[test]
fn groups_permissions_and_inheritance_from_commands() {
    let mut e = engine();
    assert!(console(&mut e, "creategroup vip").contains("Group vip created."));
    assert!(console(&mut e, "group vip create").contains("already exists"));
    console(&mut e, "group admin create");
    assert!(
        console(&mut e, "group vip permission set minecraft:command.gamemode")
            .contains("Set minecraft:command.gamemode to true")
    );
    console(&mut e, "group admin parent add vip");
    console(&mut e, "group admin setweight 100");
    assert!(console(&mut e, "user Steve parent add admin").contains(&format!("Steve ({STEVE}) now inherits admin.")));
    assert_eq!(e.decide(STEVE, "minecraft:command.gamemode", "world", 0, 1_000), Some(true));
    // a loop is refused
    assert!(console(&mut e, "group vip parent add admin").contains("loop"));
    // unknown groups and bad nodes
    assert!(console(&mut e, "user Steve parent add nope").contains("does not exist"));
    assert!(console(&mut e, "group vip permission set a..b").contains("empty part"));
    assert!(console(&mut e, "group vip permission set x maybe").contains("not true or false"));
    // check explains where the answer comes from
    let check = console(&mut e, "user Steve permission check minecraft:command.gamemode");
    assert!(check.contains("true") && check.contains("group vip"), "{check}");
    assert!(console(&mut e, "user Steve permission check other.node").contains("not set"));
    // the group check shows what a member gets
    assert!(console(&mut e, "group admin permission check minecraft:command.gamemode").contains("true"));
    // unset and clear
    assert!(console(&mut e, "group vip permission unset minecraft:command.gamemode").contains("Removed"));
    assert!(console(&mut e, "group vip permission unset minecraft:command.gamemode").contains("has no"));
    assert_eq!(e.decide(STEVE, "minecraft:command.gamemode", "world", 0, 1_000), None);
}

#[test]
fn data_survives_reloading_the_store() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "group vip permission set fly true world=nether");
    console(&mut e, "user Steve parent add vip");
    e.reload_data();
    assert_eq!(e.decide(STEVE, "fly", "nether", 0, 1_000), Some(true));
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 1_000), None);
}

#[test]
fn players_need_permissions_and_see_only_their_commands() {
    let mut e = engine();
    let p = steve();
    // nothing allowed: the help is empty and commands are refused
    assert!(run_as(&mut e, &p, "help", 0).contains("no commands you can use"));
    assert!(run_as(&mut e, &p, "creategroup x", 0).contains("permission"));
    assert!(run_as(&mut e, &p, "user Steve info", 0).contains("permission"));
    // operators of level 3 may do everything by default
    let op = Who::player("Steve", STEVE, 3, "world");
    assert!(run_as(&mut e, &op, "creategroup x", 0).contains("created"));
    // grant only user info
    console(&mut e, "user Steve permission set pumbo.perms.user.info");
    let help = run_as(&mut e, &p, "help", 0);
    assert!(help.contains("user <player>") && !help.contains("creategroup"), "{help}");
    assert!(run_as(&mut e, &p, "user Steve info", 0).contains("Rank"));
    assert!(run_as(&mut e, &p, "user Steve permission set x", 0).contains("permission"));
    // a wildcard over the plugin
    console(&mut e, "user Steve permission set pumbo.perms.*");
    assert!(run_as(&mut e, &p, "creategroup y", 0).contains("created"));
    // the Pumpkin namespace spelling is the same node
    console(&mut e, "user Steve permission set pumboperms:group.create false");
    assert!(run_as(&mut e, &p, "creategroup z", 0).contains("permission"));
    assert!(command::allowed_any(&mut e, &p, 0));
    assert!(!command::allowed_any(&mut e, &Who::player("Alex", ALEX, 0, "world"), 0));
}

#[test]
fn temporary_permissions_and_parents() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    let out = console(&mut e, "user Steve permission settemp fly true 10s");
    assert!(out.contains("for 10s"), "{out}");
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 5_000), Some(true));
    assert_eq!(e.decide(STEVE, "fly", "world", 0, 11_000), None);
    assert!(console(&mut e, "user Steve permission settemp fly").contains("Usage"));
    assert!(console(&mut e, "user Steve permission settemp fly true nope").contains("Invalid time"));
    console(&mut e, "user Steve parent addtemp vip 1m");
    assert_eq!(e.decide(STEVE, "group.vip", "world", 0, 50_000), Some(true));
    assert_eq!(e.decide(STEVE, "group.vip", "world", 0, 62_000), None);
    assert_eq!(e.expire(62_000), (true, Ok(())));
    assert!(e.perms.user(STEVE).is_none_or(|u| u.data.is_empty()));
}

#[test]
fn log_and_actions() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    let out = command::run(&mut e, &Who::console(), &split_args("group vip permission set fly"), &env(5_000));
    let entry = out.log.clone().unwrap();
    assert_eq!(
        (entry.kind.as_str(), entry.target.as_str(), entry.action.as_str()),
        ("group", "vip", "permission set fly")
    );
    assert!(out.changed);
    let log = run_as(&mut e, &Who::console(), "log", 65_000);
    assert!(log.contains("1m ago Console » group vip permission set fly"), "{log}");
    assert!(command::notification(&e, &entry).plain().contains("Console changed group vip: permission set fly"));
    let out = command::run(&mut e, &Who::console(), &split_args("reload"), &env(0));
    assert_eq!(out.action, Some(Action::Reload));
    let out = command::run(&mut e, &Who::console(), &split_args("export backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Export("backup1".into())));
    let out = command::run(&mut e, &Who::console(), &split_args("export ../x"), &env(0));
    assert!(out.action.is_none() && out.reply.plain().contains("not a valid file name"));
    let out = command::run(&mut e, &Who::console(), &split_args("import backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Import("backup1".into())));
}

#[test]
fn broken_database_refuses_changes() {
    let lang = Lang::load(&[COMMON, LANG], "en", None).0;
    let mut e = Engine::new(Err(pumbo_common::store::StoreError::new("broken")), CoreCfg::default(), lang);
    assert!(console(&mut e, "creategroup vip").contains("database is not available"));
    assert!(console(&mut e, "version").contains("broken"));
}

#[test]
fn clear_refuses_extra_arguments() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve parent add vip");
    console(&mut e, "user Steve meta set color gold");
    assert!(console(&mut e, "user Steve clear info").contains("Usage"));
    assert!(console(&mut e, "user Steve info extra").contains("Usage"));
    assert!(console(&mut e, "user Steve meta unset color extra").contains("Usage"));
    let data = &e.perms.user(STEVE).unwrap().data;
    assert_eq!((data.parents.len(), data.meta.len()), (1, 1));
}

#[test]
fn user_replies_show_uuid() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    assert!(console(&mut e, "user steve parent add vip").contains(&format!("Steve ({STEVE})")));
    assert!(console(&mut e, "user Steve permission set fly").contains(&format!("Steve ({STEVE})")));
    let nameless = "00000000-0000-0000-0000-0000000000e1";
    let out = console(&mut e, &format!("user {nameless} parent add vip"));
    assert_eq!(out.matches(nameless).count(), 1, "{out}");
}

#[test]
fn group_node_cannot_be_set_as_permission() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    for line in [
        "user Steve permission set group.vip true",
        "user Steve permission settemp group.vip 1h",
        "user Steve permission unset group.vip",
        "group vip permission set group.admin",
    ] {
        let out = console(&mut e, line);
        assert!(out.contains("is a group, not a permission") && out.contains("parent add"), "{line}: {out}");
    }
    assert!(e.perms.user(STEVE).is_none_or(|u| u.data.permissions.is_empty()));
    let json = r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip","data":{"permissions":[{"node":"group.admin","value":true}]}}],"users":[],"tracks":[]}"#;
    let snap = export::parse(json).unwrap();
    assert!(snap.groups[0].data.permissions.is_empty());
    assert!(
        snap.fixed.iter().any(|f| f.contains("group.admin left out") && f.contains("parent add admin")),
        "{:?}",
        snap.fixed
    );
}

#[test]
fn dropped_wildcard_shows_in_permission_check() {
    let mut e = engine();
    console(&mut e, "creategroup mod");
    console(&mut e, "group mod permission set pumbo.bans.* true");
    console(&mut e, "group mod permission set pumbo.bans.exempt.* false");
    console(&mut e, "user Steve parent add mod");
    let line = "not sent to Pumpkin: pumbobans:* hides pumbobans:exempt.*";
    let out = console(&mut e, "user Steve permission check pumbo.bans.limit.helper");
    assert!(out.contains(line), "{out}");
    assert!(console(&mut e, "user Steve info").contains(line));
    let out = console(&mut e, "user Steve permission check minecraft:command.op");
    assert!(!out.contains("not sent"), "only wildcards over the node: {out}");
}

/// What a user gets at a place, comparable.
fn decided(p: &mut crate::perms::Perms, uuid: &str, q: &crate::context::Query, now: u64) -> String {
    let eff = p.effective(uuid, q, now);
    let mut nodes: Vec<(&str, bool)> = eff.nodes().collect();
    nodes.sort();
    format!("{nodes:?} {:?} {} {:?} {:?} {:?}", eff.groups, eff.primary, eff.prefix, eff.suffix, eff.meta)
}

/// Decisions are kept per user and dropped only for the users a change can
/// reach: after every kind of change the kept answer equals a fresh one.
#[test]
fn cache_follows_group_change() {
    use crate::context::Query;
    let mut e = engine();
    const NEW: &str = "00000000-0000-0000-0000-00000000000c";
    const SCOPED: &str = "00000000-0000-0000-0000-00000000000d";
    e.seen(NEW, "Newbie", true, 0).unwrap();
    e.seen(SCOPED, "Scoped", true, 0).unwrap();
    let places = [Query::default(), Query::new("lobby", "hubs", "world"), Query::new("survival", "", "nether")];
    let users = [STEVE, ALEX, NEW, SCOPED];
    let steps = [
        "creategroup vip",
        "creategroup mod",
        "group vip permission set a.b",
        "user Steve parent add vip",
        "user Alex parent add mod",
        "user Scoped parent add vip server=lobby",
        "group mod parent add vip",
        "group vip permission set a.* false server=lobby",
        "group default permission set d.x",
        "user Newbie parent add later",
        "creategroup later",
        "group later permission set l.x",
        "group mod parent add later",
        "group vip meta setprefix 10 [V]",
        "group mod meta addprefix 20 [M]",
        "group vip meta set color gold",
        "group mod setweight 5",
        "group vip setweight 10",
        "user Steve permission set fly true world=nether",
        "group vip permission settemp t.x true 1m",
        "user Alex parent addtemp vip 1m",
        "createtrack ranks",
        "track ranks append vip",
        "track ranks append mod",
        "user Newbie promote ranks",
        "group vip rename vip2",
        "group mod parent remove later",
        "group default permission unset d.x",
        "group later permission set group.vip2 false",
        "deletegroup later",
        "user Steve parent remove vip2",
        "group vip2 permission clear",
        "deletegroup mod",
        "user Alex clear",
        "creategroup base",
        "group default parent add base",
        "group base permission set b.x",
        "group base permission settemp b.t true 1m",
    ];
    for (i, step) in steps.iter().enumerate() {
        let now = 1_000 + i as u64;
        for u in users {
            for q in &places {
                e.perms.effective(u, q, now);
            }
        }
        let reply = console(&mut e, step);
        // A copy starts without kept decisions.
        let mut fresh = e.perms.clone();
        for u in users {
            for q in &places {
                assert_eq!(
                    decided(&mut e.perms, u, q, now),
                    decided(&mut fresh, u, q, now),
                    "after `{step}` ({reply}): {u} at {q:?}"
                );
            }
        }
    }
    // A temporary node ends without a change: the kept answer ends with it.
    assert_eq!(e.perms.check(ALEX, "t.x", &places[0], 70_000), None);
    assert_eq!(e.perms.check(SCOPED, "b.t", &places[0], 2_000), Some(true));
    assert_eq!(e.perms.check(SCOPED, "b.t", &places[0], 70_000), None);
}

#[test]
fn meta_key_is_validated_and_quoted() {
    let mut e = engine();
    for bad in ["a:b", "x/y", &"k".repeat(65)] {
        let out = console(&mut e, &format!("user Steve meta set {bad} v"));
        assert!(out.contains("not a valid meta key"), "{bad}: {out}");
    }
    assert!(console(&mut e, "user Steve meta set Rank.Color-2_x gold").contains("Set rank.color-2_x"));
    assert!(console(&mut e, "user Steve meta unset rank.color-2_x extra").contains("Usage"));
    // The removal a click puts into the chat box takes exactly that entry.
    let text = r#"&c[A "q" \x\] "#;
    console(&mut e, &format!("user Steve meta addprefix 5 {}", quote(text)));
    console(&mut e, "user Steve meta addprefix 5 other");
    // A key an import brought, outside the rule for new keys.
    let odd = crate::model::MetaNode {
        kind: crate::model::MetaKind::Meta,
        key: "odd\"key".into(),
        priority: 0,
        value: "1".into(),
        context: crate::context::Contexts::global(),
        expiry: None,
    };
    e.perms.edit_user(STEVE, "Steve", |d| d.meta.push(odd));
    let out = command::run(&mut e, &Who::console(), &split_args("user Steve meta info"), &env(0));
    let mut runs = clicks(&out.reply);
    runs.dedup();
    for c in runs {
        if c.join(" ").contains("removeprefix 5 &c") || c.join(" ").contains("unset odd") {
            let out = command::run(&mut e, &Who::console(), &c, &env(0)).reply.plain();
            assert!(out.contains("Removed"), "{c:?}: {out}");
        }
    }
    let left: Vec<String> = e.perms.user(STEVE).unwrap().data.meta.iter().map(|m| m.value.clone()).collect();
    assert_eq!(left, ["gold", "other"], "only the clicked entries went");
}

/// [`split_args`] backwards, as a test writes a quoted word.
fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

#[test]
fn settemp_refuses_existing_expiry() {
    let mut e = engine();
    console(&mut e, "user Steve permission settemp fly true 10m");
    let out = console(&mut e, "user Steve permission settemp fly true 1h");
    assert!(
        out.contains("already has an end time") && out.contains("/pp user Steve permission unsettemp fly"),
        "{out}"
    );
    let p = e.perms.user(STEVE).unwrap().data.permissions.clone();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].expiry, Some(1_000 + 600_000), "the old end time stays");
    // Another context is another entry; after unsettemp the new time goes in.
    assert!(console(&mut e, "user Steve permission settemp fly true 1h server=lobby").contains("for 1h"));
    console(&mut e, "user Steve permission unsettemp fly");
    assert!(console(&mut e, "user Steve permission settemp fly true 1h").contains("for 1h"));
}

/// `group=a` and `group=b` score the same, so on a server in both groups the
/// order of [`crate::perms`] decides: false wins the tie.
#[test]
fn group_contexts_tie_resolves_to_false() {
    let mut e = engine();
    console(&mut e, "user Steve permission set fly true group=a");
    console(&mut e, "user Steve permission set fly false group=b");
    console(&mut e, "user Steve permission set walk false group=a");
    console(&mut e, "user Steve permission set walk true group=b");
    let q = crate::context::Query::at("", vec!["a".into(), "b".into()], "");
    assert_eq!(e.perms.check(STEVE, "fly", &q, 0), Some(false));
    assert_eq!(e.perms.check(STEVE, "walk", &q, 0), Some(false));
}
