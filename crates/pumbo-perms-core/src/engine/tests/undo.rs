//! A change that cannot be saved, undone from the records it touched.

use super::*;
use crate::command::{self, Env, Who, split_args};
use crate::export::{self, Snapshot};
use crate::model::User;

fn console(e: &mut Engine, line: &str) -> String {
    let nobody = |_: &str| -> Option<KnownName> { None };
    let env = Env { now: 1_000, online: &nobody, version: "0", version_rows: Vec::new() };
    command::run(e, &Who::console(), &split_args(line), &env).reply.plain()
}

/// The export of what memory holds and every user record, also one without data.
fn held(e: &Engine) -> (Snapshot, Vec<User>) {
    let mut users: Vec<User> = e.perms.users().cloned().collect();
    users.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    (export::snapshot(&e.perms, 0), users)
}

#[test]
fn failed_save_leaves_what_the_disk_has() {
    let backend = pumbo_common::store::FaultyBackend::default();
    let writes = backend.fail_writes.clone();
    let store = PermsStore::new(pumbo_common::store::Store::new(backend)).unwrap();
    let mut e = Engine::new(Ok(store), CoreCfg::default(), Lang::load(&[COMMON, crate::LANG], "en", None).0);
    e.seen(U, "Steve", true, 0).unwrap();
    for line in [
        "creategroup vip",
        "creategroup mod",
        "group vip permission set vip.chat",
        "user Steve parent add vip",
        "createtrack ranks",
        "track ranks append vip",
        "track ranks append mod",
    ] {
        console(&mut e, line);
    }
    console(&mut e, "user Steve permission set fly");
    let q = e.query("");
    for line in [
        "user Steve permission set build",
        "user Steve permission unset fly",
        "user Steve parent remove vip",
        "user Steve clear",
        "user Steve promote ranks",
        "group vip permission set vip.chat false",
        "group vip rename gold",
        "deletegroup vip",
        "creategroup new",
        "track ranks append default",
        "deletetrack ranks",
        "user 00000000-0000-0000-0000-000000000009 permission set fly",
    ] {
        let before = held(&e);
        let answers = (e.perms.check(U, "fly", &q, 1_000), e.perms.check(U, "vip.chat", &q, 1_000));
        writes.store(1, std::sync::atomic::Ordering::SeqCst);
        let reply = console(&mut e, line);
        assert!(reply.contains("Could not save the change: write failed"), "{line}: {reply}");
        assert_eq!(held(&e), before, "{line}: memory is as before");
        assert_eq!((e.perms.check(U, "fly", &q, 1_000), e.perms.check(U, "vip.chat", &q, 1_000)), answers, "{line}");
        e.reload_data();
        assert_eq!(held(&e), before, "{line}: memory is what the disk has");
    }
}
