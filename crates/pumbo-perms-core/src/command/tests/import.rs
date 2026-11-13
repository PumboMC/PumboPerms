//! Imports from a file or a source, re-imports and pending entries at a join.

use super::*;

#[test]
fn import_replaces_everything() {
    let mut e = engine();
    console(&mut e, "creategroup vip");
    console(&mut e, "user Steve parent add vip");
    let dump = crate::export::dump(&e.perms, 0);
    console(&mut e, "deletegroup vip");
    console(&mut e, "creategroup other");
    let out = command::import(&mut e, &Who::console(), "b", &dump, "before", 9);
    assert!(out.reply.plain().contains("Imported 2 groups, 1 users"), "{}", out.reply.plain());
    assert!(e.perms.group("vip").is_some() && e.perms.group("other").is_none());
    e.reload_data();
    assert_eq!(e.decide(STEVE, "group.vip", "", 0, 10), Some(true));
    let bad = command::import(&mut e, &Who::console(), "b", "{}", "before", 9);
    assert!(bad.reply.plain().contains("Import failed"));
    assert!(e.perms.group("vip").is_some(), "a failed import changes nothing");
}

#[test]
fn imported_names_do_not_reach_click_commands() {
    let mut e = engine();
    let json = format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"default"}},{{"name":"vip"}},{{"name":"admin"}}],
          "users":[{{"uuid":"{ALEX}","name":"Steve parent add admin","data":{{"parents":[{{"group":"vip"}}]}}}}]}}"#
    );
    command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    let out = command::run(&mut e, &Who::console(), &split_args("group vip listmembers"), &env(1_000));
    let runs = clicks(&out.reply);
    assert!(!runs.is_empty(), "{}", out.reply.plain());
    assert!(runs.iter().all(|c| !c.join(" ").contains("parent add")), "{runs:?}");
}

#[test]
fn import_contexts_are_canonical() {
    let mut e = engine();
    let snap = export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"vip","data":{"permissions":[
            {"node":"fly","value":false,"context":{"Server":"Survival"}},
            {"node":"fly","value":true},
            {"node":"build","value":true,"context":{"planet":"mars"}}]}}]}"#,
    )
    .unwrap();
    let nodes = &snap.groups[0].data.permissions;
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert!(nodes.iter().any(|p| p.node == "fly" && !p.value && p.context.get("server") == Some("survival")));
    assert!(snap.fixed.iter().any(|f| f.contains("build") && f.contains("planet")), "{:?}", snap.fixed);
    let json = serde_json::to_string(&snap).unwrap();
    command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    console(&mut e, "user Steve parent add vip");
    let q = crate::context::Query::new("Survival", "", "");
    assert_eq!(e.perms.check(STEVE, "fly", &q, 0), Some(false), "the deny works on Survival");
    assert_eq!(e.perms.check(STEVE, "fly", &crate::context::Query::default(), 0), Some(true));
}

#[test]
fn import_drops_parent_without_group() {
    let snap = export::parse(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","data":{{"parents":[{{"group":"ghost"}}]}}}}],
          "users":[{{"uuid":"{STEVE}","name":"Steve","data":{{"parents":[{{"group":"vip"}},{{"group":"default"}},{{"group":"ghost"}}]}}}}]}}"#
    ))
    .unwrap();
    assert!(snap.groups[0].data.parents.is_empty());
    let parents: Vec<&str> = snap.users[0].data.parents.iter().map(|p| p.group.as_str()).collect();
    assert_eq!(parents, vec!["vip", "default"]);
    assert_eq!(snap.fixed.iter().filter(|f| f.contains("parent \"ghost\"")).count(), 2, "{:?}", snap.fixed);
}

/// A deny group with a Polish name: preview, run, the ledger, a run of the
/// same file and undo all use its new name, and its member keeps the deny.
#[test]
fn deny_group_with_a_polish_name_keeps_denying() {
    const BAD_PLAYER: &str = "ŻółwiaGęśŚliczna_bardzo_długa_nazwa_z_ogonkami";
    let mut e = engine();
    let json = serde_json::json!({
        "format": "pumboperms", "version": 1,
        "groups": [
            {"name": "vip", "data": {"permissions": [{"node": "fly", "value": true}]}},
            {"name": "Zakaz Latania", "weight": 10, "data": {"permissions": [{"node": "fly", "value": false}]}},
        ],
        "users": [{"uuid": STEVE, "name": "Steve", "data": {"parents": [{"group": "vip"}, {"group": "Zakaz Latania"}]}}],
        "by-name": [{"name": BAD_PLAYER, "data": {"parents": [{"group": "vip"}]}}],
    });
    let snap = export::parse(&json.to_string()).unwrap();
    let source = |e: &mut Engine, fingerprint: &str, mode| {
        import_source(e, &Who::console(), "file", Ok((fingerprint.into(), snap.clone())), mode, 1)
    };
    let preview = source(&mut e, "f1", SourceMode::Preview).reply.plain();
    let run = source(&mut e, "f1", SourceMode::Run);
    for reply in [&preview, &run.reply.plain()] {
        assert!(reply.contains("2 entries of the file were fixed or left out"), "{reply}");
        assert!(reply.contains("\"Zakaz Latania\" imported as zakaz-latania"), "{reply}");
        assert!(reply.contains(&format!("{BAD_PLAYER:?}")), "{reply}");
    }
    assert!(preview.contains("Steve in zakaz-latania"), "{preview}");
    let fly = |e: &mut Engine| e.perms.check(STEVE, "fly", &crate::context::Query::default(), 0);
    assert_eq!(fly(&mut e), Some(false), "the deny holds");
    let ledger = e.ledger("file").unwrap();
    assert!(ledger.pending.is_empty(), "the bad player waits for no join");
    assert!(format!("{:?}", ledger.entries).contains("zakaz-latania"), "{:?}", ledger.entries);
    let again = source(&mut e, "f2", SourceMode::Run).reply.plain();
    assert!(again.contains("0 entries added") && !again.contains("no longer has"), "{again}");
    assert_eq!(e.ledger("file").unwrap().entries, ledger.entries, "the same file, the same entries");
    let undo = import_source(&mut e, &Who::console(), "file", Err(String::new()), SourceMode::Undo, 2);
    assert!(undo.changed, "{}", undo.reply.plain());
    assert!(e.perms.group("zakaz-latania").is_none() && e.perms.group("vip").is_none());
    assert_eq!(fly(&mut e), None);
}

/// A whole-file import keeps every rung of a track over a group with a
/// Polish name, so a promotion goes where it went before.
#[test]
fn import_keeps_every_rung_of_a_track() {
    let mut e = engine();
    let json = serde_json::json!({
        "format": "pumboperms", "version": 1,
        "groups": [{"name": "Początkujący"}, {"name": "vip"}],
        "tracks": [{"name": "ranks", "groups": ["default", "Początkujący", "vip"]}],
    });
    let out = command::import(&mut e, &Who::console(), "x", &json.to_string(), "before", 1);
    assert!(out.reply.plain().contains("\"Początkujący\" imported as poczatkujacy"), "{}", out.reply.plain());
    assert_eq!(e.perms.track("ranks").unwrap().groups, ["default", "poczatkujacy", "vip"]);
    console(&mut e, "promote Steve ranks");
    let parents = &e.perms.user(STEVE).unwrap().data.parents;
    assert!(parents.len() == 1 && parents[0].group == "poczatkujacy", "{parents:?}");
}

#[test]
fn import_drops_duplicates_and_control_chars() {
    let mut e = engine();
    let long = "x".repeat(300);
    let json = format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","display_name":"VIP\n[Owner] Steve","data":{{
            "permissions":[{{"node":"fly","value":true}},{{"node":"fly","value":false}}],
            "parents":[{{"group":"default"}},{{"group":"default"}}],
            "meta":[{{"kind":"prefix","priority":10,"value":"&6[VIP]\n&cfake line"}},
                    {{"kind":"meta","key":"note","value":"{long}"}},
                    {{"kind":"meta","key":"a\nfake","value":"v"}},
                    {{"kind":"suffix","priority":1,"value":" &7*"}}]}}}}]}}"#
    );
    let snap = export::parse(&json).unwrap();
    let g = &snap.groups[0];
    assert_eq!((g.data.permissions.len(), g.data.permissions[0].value), (1, false), "the last one wins");
    assert_eq!(g.data.parents.len(), 1);
    assert_eq!(g.data.meta.len(), 1, "{:?}", g.data.meta);
    assert!(g.display_name.is_empty());
    assert_eq!(snap.fixed.len(), 6, "{:?}", snap.fixed);
    let out = command::import(&mut e, &Who::console(), "x", &json, "before", 1);
    assert!(out.reply.plain().contains("6 entries of the file were fixed or left out"), "{}", out.reply.plain());
    // Commands refuse the same texts.
    assert!(console(&mut e, "group vip setdisplayname a\nb").contains("control characters"));
    assert!(console(&mut e, &format!("group vip meta set note {long}")).contains("too long"));
    assert!(console(&mut e, "group vip meta addprefix 5 \"a\u{7}\"").contains("control characters"));
    assert!(console(&mut e, "group vip meta set a\nFAKE v").contains("control characters"));
    assert!(console(&mut e, "group vip meta unset a\tb").contains("control characters"));
    assert!(console(&mut e, "group vip meta set \"a b\" v").contains("meta set <key>"));
}

#[test]
fn pending_import_binds_only_online() {
    let mut e = engine();
    let snap = export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
            "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
    )
    .unwrap();
    import_source(&mut e, &Who::console(), "file", Ok(("f".into(), snap)), SourceMode::Run, 1);
    let (offline, online) = ("00000000-0000-0000-0000-0000000000d1", "00000000-0000-0000-0000-0000000000d2");
    let out = import_pending(&mut e, "file", offline, "Mod1", false, 2).unwrap();
    assert!(!out.changed && out.reply.plain().contains("offline-mode"), "{}", out.reply.plain());
    assert!(e.perms.user(offline).is_none_or(|u| u.data.is_empty()));
    assert_eq!(e.ledger("file").unwrap().pending.len(), 1, "still waiting");
    let out = import_pending(&mut e, "file", online, "Mod1", true, 3).unwrap();
    assert!(out.changed);
    assert!(e.perms.user(online).unwrap().data.parents.iter().any(|p| p.group == "mod"));
    assert!(e.ledger("file").unwrap().pending.is_empty());
}

/// A source with `Mod1` in `mod` by nickname only.
fn mod1_source() -> export::Snapshot {
    export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
            "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
    )
    .unwrap()
}

fn in_mod(e: &Engine, uuid: &str) -> bool {
    e.perms.user(uuid).is_some_and(|u| u.data.parents.iter().any(|p| p.group == "mod"))
}

#[test]
fn reimport_keeps_first_resolved_uuid() {
    let mut e = engine();
    let (first, second) = ("00000000-0000-0000-0000-0000000000d1", "00000000-0000-0000-0000-0000000000d2");
    // Joined and left: the name is only in the database.
    e.seen(first, "Mod1", true, 0).unwrap();
    e.gone(first);
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    assert!(out.reply.plain().contains(&format!("Mod1 -> {first}")), "{}", out.reply.plain());
    assert!(in_mod(&e, first));
    // The name goes to another account, online now; the source is imported again.
    e.seen(second, "Mod1", true, 0).unwrap();
    assert_eq!(e.find_user("mod1").map(|k| k.uuid), Some(second.to_string()));
    let mut changed = mod1_source();
    changed.groups[0].weight = 5;
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), changed)), SourceMode::Run, 2);
    assert!(out.reply.plain().contains(&format!("Mod1 -> {first}")), "{}", out.reply.plain());
    assert!(in_mod(&e, first) && !in_mod(&e, second));
    let (log, _) = e.store().unwrap().log_page(1, 1).unwrap();
    assert!(log[0].action.contains(&format!("Mod1 -> {first}")), "{:?}", log[0]);
}

#[test]
fn offline_name_does_not_resolve_import() {
    let mut e = engine();
    let offline = "00000000-0000-0000-0000-0000000000d1";
    // Online now, then only in the database: an offline account binds neither way.
    e.seen(offline, "Mod1", false, 0).unwrap();
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    assert!(out.reply.plain().contains("known only by name"), "{}", out.reply.plain());
    e.gone(offline);
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), mod1_source())), SourceMode::Run, 2);
    assert!(out.reply.plain().contains("known only by name"), "{}", out.reply.plain());
    assert!(!in_mod(&e, offline));
    assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
}

#[test]
fn pending_on_join_same_on_both_adapters() {
    let player = "00000000-0000-0000-0000-0000000000d1";
    for names_checked in [true, false] {
        let mut e = engine();
        e.cfg.names_checked = names_checked;
        import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
        // Pumpkin always reports the account as offline; the proxy as Mojang says.
        let out = command::on_join(&mut e, player, "Mod1", false, 2);
        assert!(out.len() == 1 && !out[0].changed && !in_mod(&e, player), "{out:?}");
        assert!(out[0].reply.plain().contains("got nothing"), "{:?}", out[0].reply.plain());
        assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
    }
    let mut e = engine();
    import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    let out = command::on_join(&mut e, player, "Mod1", true, 2);
    assert!(out.len() == 1 && out[0].changed && in_mod(&e, player), "{out:?}");
    assert!(command::on_join(&mut e, player, "Mod1", true, 3).is_empty(), "nothing waits any more");
}

#[test]
fn pumpkin_import_reply_names_players_who_get_nothing() {
    let mut e = engine();
    e.cfg.names_checked = false;
    let out = import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), mod1_source())), SourceMode::Run, 1);
    let reply = out.reply.plain();
    assert!(reply.contains("get nothing here; give them their ranks with /pp: Mod1"), "{reply}");
    assert!(!reply.contains("premium"), "{reply}");
}

#[test]
fn import_file_needs_console() {
    let mut e = engine();
    let op = Who::player("Steve", STEVE, 4, "world");
    let out = command::run(&mut e, &op, &split_args("import backup1"), &env(0));
    assert!(out.action.is_none() && out.reply.plain().contains("Only the console"), "{}", out.reply.plain());
    let out = command::run(&mut e, &op, &split_args("import preview file"), &env(0));
    assert_eq!(out.action, Some(Action::Source("file".into(), SourceMode::Preview)));
    let out = command::run(&mut e, &Who::console(), &split_args("import backup1"), &env(0));
    assert_eq!(out.action, Some(Action::Import("backup1".into())));
}

/// Re-imports, storage errors and subcommands that take only temporary entries.
mod storage_and_import {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pumbo_common::store::{FaultyBackend, Store};

    use super::*;
    use crate::store::PermsStore;

    struct Faults {
        reads: Arc<AtomicU32>,
        writes: Arc<AtomicU32>,
    }

    fn faulty_engine() -> (Engine, Faults) {
        let backend = FaultyBackend::default();
        let faults = Faults { reads: backend.fail_reads.clone(), writes: backend.fail_writes.clone() };
        let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
        let mut e = Engine::new(PermsStore::new(Store::new(backend)), CoreCfg::default(), lang);
        e.seen(STEVE, "Steve", true, 0).unwrap();
        (e, faults)
    }

    /// A source with helper, builder (weight 10) and mod, each with a node.
    fn ranks_source(extra: &str) -> export::Snapshot {
        export::parse(&format!(
            r#"{{"format":"pumboperms","version":1,"groups":[
                {{"name":"helper","data":{{"permissions":[{{"node":"a.helper","value":true}}]}}}},
                {{"name":"builder","weight":10,"data":{{"permissions":[{{"node":"a.builder","value":true}}]}}}},
                {{"name":"mod","data":{{"permissions":[{{"node":"a.mod","value":true}}{extra}]}}}}],
              "users":[{{"uuid":"{STEVE}","name":"Steve","data":{{"parents":[{{"group":"helper"}}]}}}}]}}"#
        ))
        .unwrap()
    }

    fn import(e: &mut Engine, fingerprint: &str, snap: export::Snapshot) {
        let out = import_source(e, &Who::console(), "file", Ok((fingerprint.into(), snap)), SourceMode::Run, 1_000);
        assert!(out.changed, "import did not run: {}", out.reply.plain());
    }

    #[test]
    fn reimport_keeps_tracks_inheritance_and_weight() {
        let mut e = engine();
        import(&mut e, "f1", ranks_source(""));
        console(&mut e, "createtrack ranks");
        for g in ["helper", "builder", "mod"] {
            console(&mut e, &format!("track ranks append {g}"));
        }
        console(&mut e, "creategroup trial");
        console(&mut e, "group trial parent add builder");
        console(&mut e, "group builder setweight 20");
        assert_eq!(e.perms.track("ranks").unwrap().groups.len(), 3);
        import(&mut e, "f2", ranks_source(r#",{"node":"a.new","value":true}"#));
        let track = e.perms.track("ranks").unwrap().groups.clone();
        let trial_inherits = e.perms.group("trial").unwrap().data.parents.iter().any(|p| p.group == "builder");
        let weight = e.perms.group("builder").unwrap().weight;
        assert_eq!((track, trial_inherits, weight), (vec!["helper".into(), "builder".into(), "mod".into()], true, 20));
    }

    #[test]
    fn reimport_keeps_admin_temporary_parent() {
        let mut e = engine();
        import(&mut e, "f1", ranks_source(""));
        console(&mut e, "user Steve parent addtemp helper 1d");
        assert_eq!(e.perms.user(STEVE).unwrap().data.parents.len(), 2);
        import(&mut e, "f2", ranks_source(r#",{"node":"a.new","value":true}"#));
        let parents = &e.perms.user(STEVE).unwrap().data.parents;
        assert!(parents.iter().any(|p| p.group == "helper" && p.expiry.is_some()), "{parents:?}");
        assert!(parents.iter().any(|p| p.group == "helper" && p.expiry.is_none()), "{parents:?}");
    }

    #[test]
    fn import_log_cut_on_char_boundary() {
        for n in [600, 601] {
            let mut e = engine();
            console(&mut e, "user Steve permission set fly false");
            let snap = export::parse(&format!(
                r#"{{"format":"pumboperms","version":1,"users":[{{"uuid":"{STEVE}","name":"{}",
                  "data":{{"permissions":[{{"node":"fly","value":true}}]}}}}]}}"#,
                "ż".repeat(n)
            ))
            .unwrap();
            import(&mut e, "f1", snap);
            let (log, _) = e.store().unwrap().log_page(1, 1).unwrap();
            assert!(log[0].action.len() <= 1000, "{n}: {} bytes", log[0].action.len());
        }
    }

    #[test]
    fn noop_command_on_read_error_does_not_break_engine() {
        let (mut e, faults) = faulty_engine();
        faults.reads.store(u32::MAX, Ordering::SeqCst);
        let out = console(&mut e, "user Steve permission unset typo.node");
        faults.reads.store(0, Ordering::SeqCst);
        assert!(out.contains("has no"), "{out}");
        assert_eq!(e.broken, None);
    }

    #[test]
    fn failed_commit_and_reload_restore_memory() {
        let (mut e, faults) = faulty_engine();
        console(&mut e, "creategroup vip");
        faults.reads.store(u32::MAX, Ordering::SeqCst);
        faults.writes.store(u32::MAX, Ordering::SeqCst);
        console(&mut e, "group vip permission set fly");
        assert!(e.perms.group("vip").unwrap().data.permissions.is_empty(), "memory shows what is not on disk");
    }

    #[test]
    fn parent_removetemp_keeps_permanent_parent() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        console(&mut e, "user Steve parent add vip");
        console(&mut e, "user Steve parent addtemp vip 7d");
        assert_eq!(e.perms.user(STEVE).unwrap().data.parents.len(), 2);
        console(&mut e, "user Steve parent removetemp vip");
        let parents = &e.perms.user(STEVE).unwrap().data.parents;
        assert!(parents.iter().any(|p| p.group == "vip" && p.expiry.is_none()), "{parents:?}");
    }

    #[test]
    fn meta_removetempprefix_keeps_permanent_prefix() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        console(&mut e, "group vip meta addprefix 10 \"&6[VIP] \"");
        console(&mut e, "group vip meta addtempprefix 10 \"&6[VIP] \" 7d");
        assert_eq!(e.perms.group("vip").unwrap().data.meta.len(), 2);
        console(&mut e, "group vip meta removetempprefix 10");
        let meta = &e.perms.group("vip").unwrap().data.meta;
        assert!(meta.iter().any(|m| m.value == "&6[VIP] " && m.expiry.is_none()), "{meta:?}");
    }

    #[test]
    fn meta_unknown_affix_subcommand_is_refused() {
        let mut e = engine();
        console(&mut e, "creategroup vip");
        let out = console(&mut e, "group vip meta addfooprefix 10 x");
        assert!(out.contains("Unknown subcommand"), "{out}");
        assert!(e.perms.group("vip").unwrap().data.meta.is_empty());
    }

    #[test]
    fn reimport_keeps_parent_removed_from_player_bound_at_join() {
        let jeb = "00000000-0000-0000-0000-0000000000e1";
        let source = |weight: i32| {
            let mut snap = export::parse(
                r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
                    "by-name":[{"name":"jeb_","data":{"parents":[{"group":"mod"}]}}]}"#,
            )
            .unwrap();
            snap.groups[0].weight = weight;
            snap
        };
        let mut e = engine();
        import(&mut e, "f1", source(0));
        e.seen(jeb, "Jeb_", true, 0).unwrap();
        assert!(command::on_join(&mut e, jeb, "Jeb_", true, 2).iter().any(|o| o.changed));
        console(&mut e, "user Jeb_ parent remove mod");
        assert!(!in_mod(&e, jeb));
        let out = import_source(&mut e, &Who::console(), "file", Ok(("f2".into(), source(5))), SourceMode::Run, 3);
        assert!(!in_mod(&e, jeb), "{}", out.reply.plain());
        assert!(!out.reply.plain().contains("taken back"), "{}", out.reply.plain());
    }

    #[test]
    fn reimport_adds_no_parent_in_deleted_group() {
        let source = |members: &[(&str, &str)]| {
            let users: Vec<String> = members
                .iter()
                .map(|(uuid, name)| {
                    format!(r#"{{"uuid":"{uuid}","name":"{name}","data":{{"parents":[{{"group":"mod"}}]}}}}"#)
                })
                .collect();
            export::parse(&format!(
                r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"mod"}},{{"name":"vip"}}],"users":[{}]}}"#,
                users.join(",")
            ))
            .unwrap()
        };
        let mut e = engine();
        import(&mut e, "f1", source(&[(STEVE, "Steve")]));
        console(&mut e, "deletegroup mod");
        assert!(e.perms.group("mod").is_none());
        import(&mut e, "f2", source(&[(STEVE, "Steve"), (ALEX, "Alex")]));
        assert!(e.perms.group("mod").is_none() && !in_mod(&e, ALEX) && !in_mod(&e, STEVE));
    }

    #[test]
    fn damaged_ledger_reported_once_at_joins() {
        let store = Store::in_memory();
        store.write(|tx| tx.put(crate::store::IMPORTS, "file", b"{broken")).unwrap();
        let lang = Lang::load(&[COMMON, LANG, PLATFORM], "en", None).0;
        let mut e = Engine::new(PermsStore::new(store), CoreCfg::default(), lang);
        let first = command::on_join(&mut e, STEVE, "Steve", true, 1);
        assert!(first.len() == 1 && first[0].reply.plain().contains("damaged"), "{first:?}");
        assert!(command::on_join(&mut e, ALEX, "Alex", true, 2).is_empty());
    }
}
