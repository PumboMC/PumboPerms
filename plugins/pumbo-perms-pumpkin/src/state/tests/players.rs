//! Players joining and leaving the server.

use super::*;

#[test]
fn leave_forgets_online_name() {
    let mut rt = rt();
    rt.ready = true;
    join(&mut rt, U, 0);
    assert_eq!(rt.engine.connected(U).map(|k| k.name.as_str()), Some("Steve"));
    rt.leave(U, 0);
    assert_eq!(rt.engine.connected(U), None);
    assert_eq!(rt.engine.find_user("steve").map(|k| k.uuid), Some(U.to_string()), "the database keeps the name");
}

#[test]
fn database_opened_late_learns_the_online_names() {
    let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-names-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(format!("{}.lock", path.display()));
    let cfg = Config::default();
    let lang = Lang::load(&crate::BUNDLES, "en", None).0;
    let engine = Engine::new(Err(pumbo_common::store::StoreError::new("held")), cfg.core(), lang);
    let mut rt = Rt::with_lock(String::new(), cfg, engine, false, None, 0);
    join(&mut rt, U, 0);
    rt.retry_database(LOCK_RETRY_MS, |_| {
        let lock = DbLock::acquire(&path, LOCK_RETRY_MS).map_err(|e| e.to_string())?;
        Ok((lock, Engine::in_memory(Config::default().core(), Lang::load(&crate::BUNDLES, "en", None).0)))
    });
    assert!(rt.lock.is_some() && rt.engine.writable());
    assert_eq!(rt.engine.connected(U).map(|k| k.name.as_str()), Some("Steve"));
    rt.leave(U, LOCK_RETRY_MS);
    assert_eq!(rt.engine.find_user("steve").map(|k| k.uuid), Some(U.to_string()), "saved too");
    drop(rt);
    let _ = std::fs::remove_file(format!("{}.lock", path.display()));
}

#[test]
fn passive_or_not_ready_plans_nothing() {
    let mut rt = rt();
    assert!(join(&mut rt, U, 0).is_empty(), "before the first import pass");
    rt.ready = true;
    set_passive(true);
    give_vip(&mut rt);
    assert!(rt.plan_all(0).is_empty(), "the proxy's PumboPerms rules");
    assert!(rt.change_world(U, "nether".into(), false, 0).is_empty());
    assert_eq!(decide_import(true, false, true, None, 0), Step::Wait);
    set_passive(false);
    assert!(!rt.plan_all(0).is_empty());
}

#[test]
fn takeover_unsets_whole_catalog() {
    let mut rt = rt();
    rt.ready = true;
    join(&mut rt, U, 0);
    let handed = rt.hand_over();
    assert!(handed.contains_key(U) && rt.pushed.is_empty());
    // The proxy's PumboPerms is gone: the next pass is a first one again.
    rt.ready = false;
    let (first, plans, _) = rt.import_pass(Vec::new(), None, 0);
    assert!(first);
    let ops = ops(&mut rt, plans, U);
    let wanted = rt.pushed.get(U).unwrap();
    for node in rt.catalog.nodes().iter().filter(|n| !wanted.contains_key(*n)) {
        assert!(ops.contains(&Op::Unset(node.clone())), "{node} not unset");
    }
}

#[test]
fn plans_only_differences() {
    let mut rt = rt();
    rt.ready = true;
    give_vip(&mut rt);
    let first = join(&mut rt, U, 0);
    assert!(first.contains(&Op::Set("group.vip".into(), true)), "{first:?}");
    assert!(rt.plan_all(0).is_empty(), "nothing changed");
}

#[test]
fn leave_unsets_and_forgets_pushed() {
    let mut rt = rt();
    rt.ready = true;
    give(&mut rt, "admin", "minecraft:command.op", None);
    assert!(join(&mut rt, U, 0).contains(&Op::Set("minecraft:command.op".into(), true)));
    // The change comes while the player is leaving: its plan finds nobody to apply to.
    let (_, d) = rt
        .engine
        .perms
        .edit_user(U, "Steve", |d| d.remove_parent("admin", &pumbo_perms_core::context::Contexts::global(), false));
    rt.engine.commit(&d, None).unwrap();
    let late = rt.plan_all(0);
    let (gone, _) = rt.leave(U, 0);
    assert!(gone.contains(&Op::Unset("minecraft:command.op".into())), "{gone:?}");
    rt.applied(late);
    assert!(!rt.pushed.contains_key(U), "{:?}", rt.pushed);
    // Back again: the whole catalog is checked, the node goes even if the unset at leave failed.
    let back = join(&mut rt, U, 0);
    assert!(back.contains(&Op::Unset("minecraft:command.op".into())), "{back:?}");
}

#[test]
fn unload_clears_offline_attachments() {
    let mut rt = rt();
    rt.ready = true;
    give(&mut rt, "admin", "minecraft:command.op", None);
    join(&mut rt, U, 0);
    rt.leave(U, 0);
    // Demoted while offline, then the plugin is unloaded and loaded again.
    let (_, d) = rt
        .engine
        .perms
        .edit_user(U, "Steve", |d| d.remove_parent("admin", &pumbo_perms_core::context::Contexts::global(), false));
    rt.engine.commit(&d, None).unwrap();
    let Rt { dir, cfg, engine, .. } = rt;
    let mut rt = Rt::new(dir, cfg, engine, false);
    rt.ready = true;
    let back = join(&mut rt, U, 0);
    assert!(back.contains(&Op::Unset("minecraft:command.op".into())), "{back:?}");
    assert!(!back.contains(&Op::Set("minecraft:command.op".into(), true)));
}

#[test]
fn load_reconciles_online_attachments() {
    let earlier = rt();
    // A node an earlier run set, gone from the data since.
    earlier.engine.store().unwrap().add_host_nodes(vec![&"essentials:fly".to_string()]).unwrap();
    let mut rt = Rt::new(String::new(), earlier.cfg.clone(), earlier.engine, false);
    join(&mut rt, U, 0);
    let (first, plans, _) = rt.import_pass(Vec::new(), None, 0);
    assert!(first);
    let ops = ops(&mut rt, plans, U);
    assert!(ops.contains(&Op::Unset("essentials:fly".into())), "{ops:?}");
    // Taken off everyone online: forgotten, so the list does not only grow.
    assert!(!rt.catalog.nodes().contains("essentials:fly"));
    assert!(!rt.engine.store().unwrap().host_nodes().unwrap().contains("essentials:fly"));
}

#[test]
fn cancelled_change_world_keeps_world() {
    let mut rt = rt();
    rt.ready = true;
    let node = pumbo_perms_core::model::PermNode {
        node: "minecraft:command.fly".into(),
        value: true,
        context: pumbo_perms_core::context::Contexts::parse(&["world=nether".into()]).unwrap(),
        expiry: None,
    };
    rt.engine.perms.edit_user(U, "Steve", |d| d.set_permission(node));
    rt.refresh_catalog();
    join(&mut rt, U, 0);
    assert!(rt.change_world(U, "nether".into(), true, 0).is_empty());
    assert_eq!(rt.online[U].world, "world");
    let plans = rt.change_world(U, "nether".into(), false, 0);
    assert_eq!(rt.online[U].world, "nether");
    assert!(ops(&mut rt, plans, U).contains(&Op::Set("minecraft:command.fly".into(), true)));
    // A world change the event did not report shows at the next refresh.
    let plans = rt.refresh(&[(U.to_string(), 0, "world".into())], 0);
    assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.fly".into())));
}

#[test]
fn expiry_checked_on_entry() {
    let mut rt = rt();
    rt.ready = true;
    give(&mut rt, "vip", "minecraft:command.gamemode", Some(10_000));
    assert!(join(&mut rt, U, 0).contains(&Op::Set("minecraft:command.gamemode".into(), true)));
    assert!(rt.due_expiry(9_999).is_empty());
    // Another player joins after the time: the expired node goes for everyone.
    let other = "00000000-0000-0000-0000-000000000002";
    let plans = rt.join(other, steve(), 12_000);
    assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.gamemode".into())));
    assert!(rt.engine.perms.next_expiry().is_none(), "saved");
}

#[test]
fn expiry_due_plans_when_commit_fails() {
    let backend = pumbo_common::store::FaultyBackend::default();
    let writes = backend.fail_writes.clone();
    let store = pumbo_perms_core::store::PermsStore::new(pumbo_common::store::Store::new(backend)).unwrap();
    let cfg = Config::default();
    let engine = Engine::new(Ok(store), cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
    let mut rt = Rt::new(String::new(), cfg, engine, false);
    rt.ready = true;
    give(&mut rt, "vip", "minecraft:command.gamemode", Some(10_000));
    join(&mut rt, U, 0);
    writes.store(u32::MAX, std::sync::atomic::Ordering::SeqCst);
    let plans = rt.due_expiry(12_000);
    assert!(ops(&mut rt, plans, U).contains(&Op::Unset("minecraft:command.gamemode".into())));
    assert!(rt.logs.iter().any(|(_, l)| l.contains("trying again")), "{:?}", rt.logs);
    writes.store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(rt.due_expiry(13_000).is_empty(), "nothing new to send");
    assert!(rt.engine.perms.next_expiry().is_none(), "saved at the next call");
}

#[test]
fn alias_guard_decides_from_memory() {
    let mut rt = rt();
    let _ = rt.join(U, Online { op_level: 2, ..steve() }, 0);
    assert!(rt.alias_allowed(U, "minecraft:command.teleport", 0), "Pumpkin's Op(2)");
    assert!(!rt.alias_allowed(U, "minecraft:command.banip", 0), "banip is Op(3)");
    rt.engine.perms.edit_user(U, "Steve", |d| {
        d.set_permission(pumbo_perms_core::model::PermNode {
            node: "minecraft:command.teleport".into(),
            value: false,
            context: pumbo_perms_core::context::Contexts::global(),
            expiry: None,
        })
    });
    assert!(!rt.alias_allowed(U, "minecraft:command.teleport", 0), "the data decides first");
    assert!(!rt.alias_allowed("00000000-0000-0000-0000-000000000009", "minecraft:command.teleport", 0));
}

#[test]
fn command_lists_resent_at_most_once_a_second() {
    let mut rt = rt();
    const OTHER: &str = "00000000-0000-0000-0000-000000000002";
    assert_eq!(rt.resends(vec![U.into(), OTHER.into()], 1_000), [U, OTHER]);
    // Three more changes within the second: one list, when the second is over.
    for at in [1_100, 1_400, 1_900] {
        assert!(rt.resends(vec![U.into()], at).is_empty());
        assert!(rt.due_resends(at).is_empty());
    }
    assert_eq!(rt.due_resends(2_000), [U]);
    assert!(rt.due_resends(2_500).is_empty());
    assert_eq!(rt.resends(vec![OTHER.into()], 2_500), [OTHER]);
    // Leaving drops a waiting one.
    assert!(rt.resends(vec![OTHER.into()], 2_600).is_empty());
    rt.leave(OTHER, 2_600);
    assert!(rt.due_resends(9_000).is_empty());
}

#[test]
fn reload_reads_refresh_seconds() {
    let mut rt = rt();
    assert_eq!(rt.cfg.provider.refresh_seconds, 5);
    assert!(rt.refresh_due(100_000));
    assert!(!rt.refresh_due(104_999));
    assert!(rt.refresh_due(105_000));
    // `/pp reload` replaces the config; the same task follows the new period.
    let (cfg, _) = pumbo_common::config::load::<Config>("provider:\n  refresh-seconds: 1\n");
    rt.cfg = cfg;
    assert!(!rt.refresh_due(105_999));
    assert!(rt.refresh_due(106_000));
    assert!(rt.refresh_due(107_000));
}
