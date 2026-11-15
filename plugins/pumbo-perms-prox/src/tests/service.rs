//! Players joining and leaving the proxy.

use super::*;

/// The name the engine keeps for a player while they are online.
fn online_name(p: &PumboPerms, uuid: &str) -> Option<String> {
    p.with(|s| s.engine.connected(uuid).map(|k| k.name.clone())).unwrap()
}

#[test]
fn disconnect_forgets_online_name() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    load(&p, steve);
    let uuid = uuid_of(&players::get(steve).unwrap());
    assert_eq!(online_name(&p, &uuid), Some("Steve".into()));
    testing::with(|h| h.players.remove(&steve));
    block_on(p.on_disconnect(steve));
    assert_eq!(online_name(&p, &uuid), None);
    // The name comes from the database now.
    console(&p, "user Steve permission set fly");
    assert!(p.with(|s| s.engine.perms.user(&uuid).is_some()).unwrap(), "{}", logs());
}

#[test]
fn reconnect_before_disconnect_keeps_the_online_name() {
    let (p, _) = plugin(None);
    let first = testing::add_player("Steve", Some("lobby"));
    load(&p, first);
    let uuid = uuid_of(&players::get(first).unwrap());
    // The account joins again before the proxy reports the first session gone.
    let second = testing::add_player("Steve", Some("lobby"));
    testing::with(|h| {
        let id = h.players[&first].profile.id;
        h.players.get_mut(&second).unwrap().profile.id = id;
        h.players.remove(&first);
    });
    load(&p, second);
    block_on(p.on_disconnect(first));
    assert_eq!(online_name(&p, &uuid), Some("Steve".into()));
    block_on(p.on_disconnect(second));
    assert_eq!(online_name(&p, &uuid), None);
}

fn offline_check(p: &PumboPerms, uuid: &str) -> Option<bool> {
    let ask = CheckOffline { uuid: uuid.into(), node: "group.vip".into(), context: "global".into() };
    let answer: CheckOfflineAnswer = uncbor(&call(p, METHOD_CHECK_OFFLINE, cbor(&ask).unwrap()).unwrap()).unwrap();
    answer.value
}

fn uuids(n: u64) -> Vec<String> {
    (0..n).map(|i| Uuid::from_high_low(7, i).to_string()).collect()
}

#[test]
fn check_offline_reads_the_record_from_the_database() {
    let (p, _) = plugin(None);
    let players = uuids(2);
    in_vip(&p, &players[..1]);
    assert_eq!(offline_check(&p, &players[0]), Some(true));
    assert_eq!(offline_check(&p, &players[1]), None);
    assert!(p.with(|s| s.engine.connected(&players[0]).is_none()).unwrap());
}

#[test]
fn disconnect_lets_the_record_go() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    let uuid = uuid_of(&players::get(steve).unwrap());
    let mut all = uuids(100);
    all.push(uuid.clone());
    in_vip(&p, &all);
    assert!(has(&load(&p, steve), "group.vip", Context::Global, true));
    let known = |p: &PumboPerms| p.with(|s| s.engine.perms.knows(&uuid)).unwrap();
    for other in &all[..100] {
        assert_eq!(offline_check(&p, other), Some(true));
    }
    assert!(known(&p), "online");
    testing::with(|h| h.players.remove(&steve));
    block_on(p.on_disconnect(steve));
    assert!(known(&p), "one of the records of players who are not online now");
    for other in &all[..100] {
        offline_check(&p, other);
    }
    assert!(!known(&p), "it went back to the database");
    assert_eq!(offline_check(&p, &uuid), Some(true));
}

#[test]
fn refused_login_leaves_no_online_player() {
    use std::sync::atomic::Ordering;
    let backend = pumbo_common::store::FaultyBackend::default();
    let reads = backend.fail_reads.clone();
    let (p, _) = plugin_on(None, PermsStore::new(Store::new(backend)).unwrap());
    let steve = testing::add_player("Steve", Some("lobby"));
    let uuid = uuid_of(&players::get(steve).unwrap());
    in_vip(&p, std::slice::from_ref(&uuid));
    reads.store(1, Ordering::SeqCst);
    let refused = block_on(p.on_permission_load(players::get(steve).unwrap()));
    assert!(refused.is_err_and(|e| e.contains("not available")));
    assert!(p.with(|s| s.engine.connected(&uuid).is_none() && s.loaded.is_empty()).unwrap(), "not online");
}

#[test]
fn start_sets_the_host_hook() {
    let (p, _) = plugin(None);
    assert!(p.with(|s| s.engine.store().is_some_and(|st| st.breathe().is_some())).unwrap());
}
