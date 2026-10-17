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
