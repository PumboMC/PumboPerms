use pumbo_common::lang::Lang;
use pumbo_perms_core::engine::Engine;

use super::*;

mod import;
mod ipc;
mod players;

const U: &str = "00000000-0000-0000-0000-000000000001";

fn rt() -> Rt {
    let cfg = Config::default();
    let engine = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
    let mut rt = Rt::new(String::new(), cfg, engine, false);
    rt.engine.perms.create_group("vip").unwrap();
    rt
}

fn steve() -> Online {
    Online { name: "Steve".into(), world: "world".into(), op_level: 0 }
}

/// The changes for `uuid` in `plans`, applied as the plugin does.
fn ops(rt: &mut Rt, plans: Vec<Plan>, uuid: &str) -> Vec<Op> {
    let mine = plans.iter().filter(|p| p.uuid == uuid).flat_map(|p| p.ops.clone()).collect();
    rt.applied(plans);
    mine
}

fn join(rt: &mut Rt, uuid: &str, now: u64) -> Vec<Op> {
    let plans = rt.join(uuid, steve(), now);
    ops(rt, plans, uuid)
}

fn give(rt: &mut Rt, group: &str, node: &str, expiry: Option<u64>) {
    if rt.engine.perms.group(group).is_none() {
        rt.engine.perms.create_group(group).unwrap();
    }
    let (_, d) = rt
        .engine
        .perms
        .edit_group(group, |g| {
            g.data.set_permission(pumbo_perms_core::model::PermNode {
                node: node.into(),
                value: true,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            })
        })
        .unwrap();
    rt.engine.commit(&d, None).unwrap();
    let parent = pumbo_perms_core::model::ParentNode {
        group: group.into(),
        context: pumbo_perms_core::context::Contexts::global(),
        expiry,
    };
    let (_, d) = rt.engine.perms.edit_user(U, "Steve", |d| d.add_parent(parent));
    rt.engine.commit(&d, None).unwrap();
    rt.refresh_catalog();
}

fn give_vip(rt: &mut Rt) {
    let parent = pumbo_perms_core::model::ParentNode {
        group: "vip".into(),
        context: pumbo_perms_core::context::Contexts::global(),
        expiry: None,
    };
    rt.engine.perms.edit_user(U, "Steve", |d| d.add_parent(parent));
    rt.refresh_catalog();
}

#[test]
fn entry_refreshes_lock_while_ticks_frozen() {
    let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-lock-{}.redb", std::process::id()));
    let lock = DbLock::acquire(&path, 0).unwrap();
    let cfg = Config::default();
    let engine = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
    let mut rt = Rt::with_lock(String::new(), cfg, engine, false, Some(lock), 0);
    // No periodic task runs; a player's command 25 s later is an entry.
    assert!(rt.due_expiry(25_000).is_empty());
    assert!(DbLock::acquire(&path, 54_999).is_err(), "a second copy is refused while players use the plugin");
    // Another instance took the stale lock over: this one stops writing.
    let other = DbLock::acquire(&path, 90_000).unwrap();
    rt.due_expiry(100_000);
    assert!(rt.lock.is_none() && rt.engine.broken.is_some() && !rt.engine.writable());
    assert!(rt.errors.iter().any(|l| l.contains("taken over")), "{:?}", rt.errors);
    // Nothing reaches the file the other instance has open, a join neither.
    assert!(rt.engine.store().is_none());
    rt.join(U, steve(), 100_001);
    assert!(rt.engine.store().is_none());
    drop(other);
}

#[test]
fn database_held_elsewhere_waits_then_plans_for_online_players() {
    let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-late-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(format!("{}.lock", path.display()));
    // Steve in `group`, as the database on disk says.
    let opened = |group: &str, now: u64| -> Result<(DbLock, Engine), String> {
        let lock = DbLock::acquire(&path, now).map_err(|e| e.to_string())?;
        let cfg = Config::default();
        let mut e = Engine::in_memory(cfg.core(), Lang::load(&crate::BUNDLES, "en", None).0);
        let mut d = e.perms.create_group(group).unwrap();
        let parent = pumbo_perms_core::model::ParentNode {
            group: group.into(),
            context: pumbo_perms_core::context::Contexts::global(),
            expiry: None,
        };
        d.merge(e.perms.edit_user(U, "Steve", |d| d.add_parent(parent)).1);
        e.commit(&d, None).unwrap();
        Ok((lock, e))
    };
    // A killed server left its lock; the restart comes 1 s later.
    std::mem::forget(DbLock::acquire(&path, 0).unwrap());
    let cfg = Config::default();
    let lang = Lang::load(&crate::BUNDLES, "en", None).0;
    let engine = Engine::new(Err(pumbo_common::store::StoreError::new("held")), cfg.core(), lang);
    let mut rt = Rt::with_lock(String::new(), cfg, engine, false, None, 1_000);
    assert!(join(&mut rt, U, 1_000).is_empty());
    let (first, plans, _) = rt.import_pass(Vec::new(), None, 1_000);
    assert!(!first && plans.is_empty() && !rt.ready, "no first pass on an empty engine");
    assert!(rt.retry_database(11_000, |_| opened("vip", 11_000)).is_empty());
    assert!(!rt.import_pass(Vec::new(), None, 11_000).0 && !rt.ready && rt.lock.is_none());
    // The lock went stale: the database opens and the first pass sets it all.
    assert!(rt.retry_database(31_000, |_| opened("vip", 31_000)).is_empty(), "not ready yet");
    assert!(rt.lock.is_some() && rt.engine.broken.is_none());
    let (first, plans, _) = rt.import_pass(Vec::new(), None, 31_000);
    assert!(first && ops(&mut rt, plans, U).contains(&Op::Set("group.vip".into(), true)));
    // Another instance takes it over and changes Steve; when it lets go, the
    // database opens here again and the players get what it holds now.
    let other = DbLock::acquire(&path, 61_000).unwrap();
    rt.refresh_lock(61_000 + 1);
    assert!(rt.lock.is_none() && rt.engine.store().is_none());
    drop(other);
    let now = 61_000 + LOCK_RETRY_MS + 1;
    let plans = rt.retry_database(now, |_| opened("mvp", now));
    assert!(ops(&mut rt, plans, U).contains(&Op::Set("group.mvp".into(), true)));
}

#[test]
fn failed_open_under_lock_is_tried_again() {
    let path = std::env::temp_dir().join(format!("pumbo-perms-pumpkin-reopen-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(format!("{}.lock", path.display()));
    let cfg = Config::default();
    let lang = Lang::load(&crate::BUNDLES, "en", None).0;
    let engine = Engine::new(Err(pumbo_common::store::StoreError::new("cannot open")), cfg.core(), lang);
    let lock = DbLock::acquire(&path, 0).unwrap();
    let mut rt = Rt::with_lock(String::new(), cfg, engine, false, Some(lock), 0);
    let reopen = |now: u64| -> Result<(DbLock, Engine), String> {
        let lock = DbLock::acquire(&path, now).map_err(|e| e.to_string())?;
        Ok((lock, Engine::in_memory(Config::default().core(), Lang::load(&crate::BUNDLES, "en", None).0)))
    };
    rt.retry_database(READ_RETRY_MS, |_| reopen(READ_RETRY_MS));
    assert!(rt.engine.writable() && rt.lock.is_some(), "{:?}", rt.engine.broken);
}

#[test]
fn dropped_wildcard_is_logged_once_per_change() {
    let mut rt = rt();
    rt.ready = true;
    rt.engine
        .perms
        .edit_group("vip", |g| {
            for (node, value) in [("pumbo.bans.*", true), ("pumbo.bans.exempt.*", false)] {
                g.data.set_permission(pumbo_perms_core::model::PermNode {
                    node: node.into(),
                    value,
                    context: pumbo_perms_core::context::Contexts::global(),
                    expiry: None,
                });
            }
        })
        .unwrap();
    give_vip(&mut rt);
    let ops = join(&mut rt, U, 0);
    assert!(!ops.iter().any(|op| matches!(op, Op::Set(n, _) if n == "pumbobans:*")), "{ops:?}");
    assert!(ops.contains(&Op::Set("pumbobans:ban".into(), true)), "the catalog keeps exact Pumbo nodes");
    let logged = |rt: &Rt| rt.logs.iter().filter(|(_, l)| l.contains("pumbobans:* hides pumbobans:exempt.*")).count();
    assert_eq!(logged(&rt), 1, "{:?}", rt.logs);
    rt.plan_all(0);
    assert_eq!(logged(&rt), 1);
    rt.leave(U, 0);
    assert!(rt.hidden.is_empty(), "{:?}", rt.hidden);
    let _ = rt.join("00000000-0000-0000-0000-000000000002", steve(), 0);
    assert!(rt.hidden.is_empty(), "a player with nothing hidden keeps no entry");
}

/// 100 players online and 8000 nodes in the catalog (20 groups of 50,
/// 700 players offline with 10 nodes of their own each).
fn at_scale() -> (Rt, Vec<String>) {
    use pumbo_perms_core::context::Contexts;
    use pumbo_perms_core::model::{ParentNode, PermNode};
    let mut rt = rt();
    rt.ready = true;
    let perm = |node: String, value: bool| PermNode { node, value, context: Contexts::global(), expiry: None };
    let parent = |g: usize| ParentNode { group: format!("g{}", g % 20), context: Contexts::global(), expiry: None };
    for g in 0..20 {
        rt.engine.perms.create_group(&format!("g{g}")).unwrap();
        rt.engine
            .perms
            .edit_group(&format!("g{g}"), |gr| {
                gr.data.add_parent(ParentNode { group: "default".into(), context: Contexts::global(), expiry: None });
                for n in 0..50 {
                    let node = match n % 3 {
                        0 => format!("minecraft:command.c{g}n{n}"),
                        1 => format!("pumbo.p{g}.a{n}"),
                        _ => format!("plugin{g}:node{n}"),
                    };
                    gr.data.set_permission(perm(node, n % 7 != 0));
                }
            })
            .unwrap();
    }
    for k in 0..700 {
        rt.engine.perms.edit_user(&format!("00000000-0000-4000-8000-{k:012}"), "", |d| {
            d.add_parent(parent(k));
            for n in 0..10 {
                d.set_permission(perm(format!("own{k}:node{n}"), true));
            }
        });
    }
    rt.refresh_catalog();
    assert!(rt.catalog.len() > 8000, "{}", rt.catalog.len());
    let online: Vec<String> = (0..100).map(|k| format!("00000000-0000-4000-9000-{k:012}")).collect();
    for (k, uuid) in online.iter().enumerate() {
        rt.engine.perms.edit_user(uuid, &format!("P{k}"), |d| d.add_parent(parent(k)));
        let plans = rt.join(uuid, Online { name: format!("P{k}"), world: "world".into(), op_level: 0 }, 0);
        rt.applied(plans);
    }
    (rt, online)
}

/// An edit with 100 players online and 8000 catalog nodes: the plans come
/// from each player's own nodes, not from the whole catalog.
#[test]
#[ignore = "release benchmark"]
fn plan_after_edit_scale() {
    let rss = || {
        let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
        out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()).unwrap_or(0)
    };
    let before = rss();
    let (mut rt, online) = at_scale();
    let mut worst = std::time::Duration::ZERO;
    for round in 0..10u64 {
        let node = pumbo_perms_core::model::PermNode {
            node: "minecraft:command.c0n1".into(),
            value: round % 2 == 0,
            context: pumbo_perms_core::context::Contexts::global(),
            expiry: None,
        };
        let (_, d) = rt.engine.perms.edit_group("default", |g| g.data.set_permission(node)).unwrap();
        rt.engine.commit(&d, None).unwrap();
        let start = std::time::Instant::now();
        let plans = rt.after_change(round);
        let took = start.elapsed();
        worst = worst.max(took);
        assert_eq!(plans.len(), online.len(), "a change in default reaches everyone");
        rt.applied(plans);
        println!("plan after an edit in default, 100 players, {} catalog nodes: {took:?}", rt.catalog.len());
    }
    // One player's change plans one player.
    let (_, d) = rt.engine.perms.edit_user(&online[3], "P3", |d| {
        d.set_permission(pumbo_perms_core::model::PermNode {
            node: "plugin1:node2".into(),
            value: false,
            context: pumbo_perms_core::context::Contexts::global(),
            expiry: None,
        })
    });
    rt.engine.commit(&d, None).unwrap();
    let start = std::time::Instant::now();
    let plans = rt.after_change(20);
    println!("plan after an edit of one player: {:?}", start.elapsed());
    assert_eq!(plans.iter().map(|p| p.uuid.as_str()).collect::<Vec<_>>(), [online[3].as_str()]);
    // Native, so 8-byte pointers: the plugin in WASM needs less. Run alone.
    println!("memory: the process grew by {} MB", rss().saturating_sub(before) / 1024);
    assert!(worst < std::time::Duration::from_millis(20), "{worst:?}");
}

#[test]
fn edit_plans_only_players_it_reaches() {
    let mut rt = rt();
    rt.ready = true;
    give(&mut rt, "vip", "minecraft:command.fly", None);
    const OTHER: &str = "00000000-0000-0000-0000-000000000002";
    let plans = rt.join(OTHER, steve(), 0);
    rt.applied(plans);
    join(&mut rt, U, 0);
    rt.engine.take_affected([U, OTHER]);
    let (_, d) = rt
        .engine
        .perms
        .edit_group("vip", |g| {
            g.data.set_permission(pumbo_perms_core::model::PermNode {
                node: "minecraft:command.fly".into(),
                value: false,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            })
        })
        .unwrap();
    rt.engine.commit(&d, None).unwrap();
    let plans = rt.after_change(0);
    assert_eq!(plans.iter().map(|p| p.uuid.as_str()).collect::<Vec<_>>(), [U], "only vip members");
    assert_eq!(ops(&mut rt, plans, U), [Op::Set("minecraft:command.fly".into(), false)]);
}
