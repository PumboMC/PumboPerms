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
