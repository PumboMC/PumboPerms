//! Many players, groups and changes: time and memory.

use super::*;

/// 1 000 changes with 20 players loaded: each one reaches the proxy, and a
/// change (command, save, new set) takes well under a millisecond.
#[test]
#[ignore = "release benchmark"]
fn many_changes() {
    let (p, _) = plugin(None);
    let ids: Vec<PlayerId> = (0..20).map(|i| testing::add_player(&format!("P{i}"), Some("lobby"))).collect();
    for id in &ids {
        load(&p, *id);
    }
    console(&p, "creategroup vip");
    let start = std::time::Instant::now();
    for i in 0..500 {
        console(&p, &format!("user P{} parent add vip server=lobby", i % 20));
        console(&p, &format!("user P{} parent remove vip server=lobby", i % 20));
    }
    let took = start.elapsed();
    let sent = testing::with(|h| h.replaced.len());
    assert!(sent >= 1000, "{sent}");
    let errors = logs().matches("did not take").count();
    assert_eq!(errors, 0);
    println!("1000 changes with 20 players: {took:?}");
    assert!(took < std::time::Duration::from_millis(500), "{took:?}");
}

/// A network at scale: 10 servers, 20 groups of 50 nodes (some per server),
/// 3000 players in the database and `online` of them loaded.
fn at_scale(online: usize) -> (PumboPerms, Vec<PlayerId>) {
    use pumbo_perms_core::context::Contexts;
    use pumbo_perms_core::model::{ParentNode, PermNode};
    use pumbo_perms_core::perms::Dirty;
    // On a database file, as the proxy runs it.
    let dir = std::env::temp_dir().join(format!("pumbo-perms-scale-{}-{online}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (p, _) = plugin_on(None, PermsStore::new(Store::open(dir.join("perms.redb")).unwrap()).unwrap());
    testing::with(|h| h.servers = (0..10).map(|i| server(&format!("s{i}"), &[])).collect());
    let on = |s: usize| Contexts::parse(&[format!("server=s{s}")]).unwrap();
    let ids: Vec<PlayerId> =
        (0..online).map(|k| testing::add_player(&format!("P{k}"), Some(&format!("s{}", k % 10)))).collect();
    let uuids: Vec<String> = ids.iter().map(|id| uuid_of(&players::get(*id).unwrap())).collect();
    p.with(|s| {
        s.engine.server_groups = server_groups();
        let perms = &mut s.engine.perms;
        let mut dirty = Dirty::default();
        for g in 0..20 {
            let name = format!("g{g}");
            dirty.merge(perms.create_group(&name).unwrap());
            let (_, d) = perms
                .edit_group(&name, |gr| {
                    gr.weight = g;
                    gr.data.add_parent(ParentNode {
                        group: "default".into(),
                        context: Contexts::global(),
                        expiry: None,
                    });
                    for n in 0..50 {
                        let node = match n % 5 {
                            0 => format!("minecraft:command.c{g}n{n}"),
                            1 => format!("pumbo.p{g}.a{n}"),
                            _ => format!("plugin{g}.node{n}"),
                        };
                        let context = if n % 10 == 9 { on(n % 10) } else { Contexts::global() };
                        gr.data.set_permission(PermNode { node, value: n % 7 != 0, context, expiry: None });
                    }
                })
                .unwrap();
            dirty.merge(d);
        }
        let parent = |g: usize, context: Contexts| ParentNode { group: format!("g{}", g % 20), context, expiry: None };
        for k in 0..3000 {
            let uuid = format!("00000000-0000-4000-8000-{k:012}");
            dirty.merge(perms.edit_user(&uuid, "", |d| d.add_parent(parent(k, Contexts::global()))).1);
        }
        for (k, uuid) in uuids.iter().enumerate() {
            let (_, d) = perms.edit_user(uuid, &format!("P{k}"), |d| {
                d.add_parent(parent(k, Contexts::global()));
                d.add_parent(parent(k + 7, on(k % 10)));
            });
            dirty.merge(d);
        }
        s.engine.commit(&dirty, None).unwrap();
    });
    for id in &ids {
        load(&p, *id);
    }
    (p, ids)
}

/// A change in `default` with 300 players on 10 servers: well inside the
/// proxy's 200 ms plugin budget, natively (the median of 20 rounds; the
/// slowest is printed too).
#[test]
#[ignore = "release benchmark"]
fn push_all_scale() {
    let (p, ids) = at_scale(300);
    let mut took = Vec::new();
    for round in 0..20 {
        testing::with(|h| h.replaced.clear());
        p.with(|s| {
            let node = pumbo_perms_core::model::PermNode {
                node: "scale.extra".into(),
                value: round % 2 == 0,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            let (_, d) = s.engine.perms.edit_group("default", |g| g.data.set_permission(node)).unwrap();
            s.engine.commit(&d, None).unwrap();
        });
        let start = std::time::Instant::now();
        block_on(p.push_all());
        took.push(start.elapsed());
        assert_eq!(testing::with(|h| h.replaced.len()), ids.len(), "every player is in default");
    }
    took.sort();
    let (median, worst) = (took[took.len() / 2], took[took.len() - 1]);
    println!("push_all, 300 players on 10 servers, a change in default: median {median:?}, slowest {worst:?}");
    assert!(median < std::time::Duration::from_millis(50), "{median:?}");
}

/// A change of one player recomputes that player alone.
#[test]
fn push_all_recomputes_only_affected() {
    let (p, ids) = at_scale(30);
    testing::with(|h| h.replaced.clear());
    console(&p, "user P7 permission set only.mine");
    let sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    assert_eq!(sent, [ids[7]]);
    // A group reaches its members: g3 is the rank of P3 and P23, and a parent of P16 on s6.
    testing::with(|h| h.replaced.clear());
    console(&p, "group g3 permission set only.g3");
    let mut sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    sent.sort();
    assert_eq!(sent, [ids[3], ids[16], ids[23]]);
    // New servers: everyone is computed again, nobody gets the same set twice.
    testing::with(|h| h.replaced.clear());
    testing::with(|h| h.servers.push(server("s10", &[])));
    block_on(p.push_all());
    assert!(testing::with(|h| h.replaced.is_empty()), "nothing names s10");
    // A parent of default reaches a player who is in default only.
    let solo = testing::add_player("Solo", Some("s0"));
    load(&p, solo);
    console(&p, "creategroup base");
    console(&p, "group default parent add base");
    testing::with(|h| h.replaced.clear());
    console(&p, "group base permission set only.base");
    let sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    assert!(sent.contains(&solo) && sent.len() == ids.len() + 1, "{sent:?}");
}

/// The proxy's permissions.yml as the host sends it: 3000 players in vip and
/// `extra` more nodes in vip.
fn big_file(extra: usize) -> Vec<u8> {
    let nodes: Vec<String> =
        (0..=extra).map(|i| format!("{{\"node\":\"minecraft:command.n{i}\",\"value\":true}}")).collect();
    let users: Vec<String> = (0..3000)
        .map(|i| {
            format!("{{\"uuid\":\"00000000-0000-4000-8000-{i:012}\",\"data\":{{\"parents\":[{{\"group\":\"vip\"}}]}}}}")
        })
        .collect();
    file(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","data":{{"permissions":[{}]}}}}],"users":[{}]}}"#,
        nodes.join(","),
        users.join(",")
    ))
}

/// permissions.yml with 3000 players on a database file: the first import at
/// start and `/pp import run file` after it changed stay well inside the
/// proxy's 200 ms plugin budget, natively; reading the file, the import and
/// each portion of new sets are separate pieces of work there.
#[test]
#[ignore = "release benchmark"]
fn import_of_3000_players_scale() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-import-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = PermsStore::new(Store::open(dir.join("perms.redb")).unwrap()).unwrap();
    let (p, _) = plugin_on(Some("import:\n  follow: false\n"), store);
    let ids: Vec<PlayerId> = (0..100).map(|k| testing::add_player(&format!("P{k}"), Some("lobby"))).collect();
    for id in &ids {
        load(&p, *id);
    }
    let start = std::time::Instant::now();
    call(&p, METHOD_FILE, big_file(0)).unwrap();
    let first = start.elapsed();
    assert!(logs().contains("Imported file"), "{}", logs());
    call(&p, METHOD_FILE, big_file(1)).unwrap();
    let start = std::time::Instant::now();
    console(&p, "import run file");
    let again = start.elapsed();
    assert_eq!(p.with(|s| s.engine.perms.users().count()).unwrap(), 3000);
    println!("import of 3000 players: first {first:?}, again after a change {again:?}");
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
    // The first import includes reading the file and new sets for the 100 players.
    assert!(first < std::time::Duration::from_millis(100) && again < std::time::Duration::from_millis(50));
}

/// Resident memory of this process in KB (`ps`).
fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0)
}

/// The memory the plugin needs at scale (300 players on 10 servers, 3000 in
/// the database, an import of 3000 more), as the growth of this process: a
/// native build has 8-byte pointers, so the plugin in WASM needs less.
/// Run alone; other tests in the same process would count too.
#[test]
#[ignore = "release benchmark"]
fn memory_at_scale() {
    let before = rss_kb();
    let (p, _) = at_scale(300);
    call(&p, METHOD_FILE, big_file(0)).unwrap();
    for _ in 0..3 {
        console(&p, "group default permission set scale.extra");
        console(&p, "group default permission unset scale.extra");
        // The fake host keeps every set it got; the proxy does not.
        testing::with(|h| h.replaced.clear());
    }
    let grown = rss_kb().saturating_sub(before);
    println!("memory at scale: the process grew by {} MB", grown / 1024);
    assert!(grown < 64 * 1024, "{grown} KB");
}
