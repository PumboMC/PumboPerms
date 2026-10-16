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
