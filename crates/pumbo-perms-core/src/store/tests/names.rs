//! The name tables: records of older builds, damaged records and how often
//! name writes reach the disk.

use std::sync::{Arc, Mutex};

use pumbo_common::store::{Backend, MemoryBackend, ReadOps};

use super::*;

const A: &str = "00000000-0000-0000-0000-0000000000a1";
const B: &str = "00000000-0000-0000-0000-0000000000b2";

fn lang() -> pumbo_common::lang::Lang {
    pumbo_common::lang::Lang::load(&[pumbo_common::lang::COMMON], "en", None).0
}

fn engine(store: Store) -> crate::engine::Engine {
    crate::engine::Engine::new(PermsStore::new(store), crate::config::CoreCfg::default(), lang())
}

fn known(uuid: &str, name: &str, online: bool) -> KnownName {
    KnownName { uuid: uuid.into(), name: name.into(), online }
}

#[test]
fn names_written_by_an_older_build_resolve() {
    let store = Store::in_memory();
    store
        .write(|tx| {
            tx.put_json(NAMES, "steve", &serde_json::json!({ "uuid": A, "name": "Steve" }))?;
            tx.put_json(NAMES, "alex", &serde_json::json!({ "uuid": B, "name": "Alex", "online": true }))
        })
        .unwrap();
    let s = PermsStore::new(store).unwrap();
    let mut damaged = Vec::new();
    assert_eq!(s.find_name("STEVE", &mut damaged).unwrap(), Some(known(A, "Steve", false)));
    assert_eq!(s.find_name("alex", &mut damaged).unwrap(), Some(known(B, "Alex", true)));
    assert_eq!(s.name_of(A, &mut damaged).unwrap(), None, "older builds keep no key per account");
    assert!(damaged.is_empty(), "{damaged:?}");
    // By UUID the name comes back at the next join; then the old record stops counting.
    let mut e = engine({
        let store = Store::in_memory();
        store.write(|tx| tx.put_json(NAMES, "steve", &serde_json::json!({ "uuid": A, "name": "Steve" }))).unwrap();
        store
    });
    assert_eq!(e.find_user(A).map(|k| k.name), Some(A.to_string()));
    e.seen(A, "Steve2", false).unwrap();
    e.gone(A);
    assert_eq!(e.find_user("steve"), None);
    assert_eq!(e.find_user("steve2").map(|k| k.uuid), Some(A.to_string()));
    assert_eq!(e.find_user(A).map(|k| k.name), Some("Steve2".to_string()));
}

#[test]
fn damaged_name_record_is_skipped_in_lookup() {
    let store = Store::in_memory();
    store
        .write(|tx| {
            tx.put(NAMES, "steve", b"{broken")?;
            tx.put_json(NAMES, "alex", &serde_json::json!({ "uuid": B, "name": "Alex" }))?;
            tx.put(NAME_KEYS, B, b"[1")?;
            tx.put_json(NAMES, "nobody", &serde_json::json!({ "uuid": "not a uuid", "name": "Nobody" }))
        })
        .unwrap();
    let s = PermsStore::new(store).unwrap();
    let mut damaged = Vec::new();
    assert_eq!(s.find_name("steve", &mut damaged).unwrap(), None);
    assert_eq!(
        s.find_name("alex", &mut damaged).unwrap(),
        Some(known(B, "Alex", false)),
        "a damaged key hides nothing"
    );
    assert_eq!(s.name_of(B, &mut damaged).unwrap(), None);
    assert_eq!(s.find_name("nobody", &mut damaged).unwrap(), None);
    let key_of_b = format!("{NAME_KEYS}/{B}");
    assert_eq!(damaged, [format!("{NAMES}/steve"), key_of_b.clone(), key_of_b, format!("{NAMES}/nobody")]);
    // The database works, and the log gets one warning per record.
    let e = crate::engine::Engine::new(Ok(s), crate::config::CoreCfg::default(), lang());
    assert_eq!(e.broken, None);
    for _ in 0..3 {
        assert_eq!(e.find_user("steve"), None);
        assert_eq!(e.find_user(B).map(|k| k.name), Some(B.to_string()));
    }
    let warnings = e.take_warnings();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("perms_names/steve is damaged"), "{warnings:?}");
    assert!(warnings[1].contains(&format!("perms_name_keys/{B} is damaged")), "{warnings:?}");
    assert!(e.take_warnings().is_empty());
}

#[test]
fn damaged_name_record_is_overwritten_at_join() {
    let store = Store::in_memory();
    store
        .write(|tx| {
            tx.put(NAMES, "steve", b"{broken")?;
            tx.put(NAME_KEYS, A, b"{broken")
        })
        .unwrap();
    let mut e = engine(store);
    assert_eq!(e.seen(A, "Steve", true), Ok(None));
    assert_eq!(e.take_warnings().len(), 2);
    e.gone(A);
    assert_eq!(e.find_user("steve"), Some(known(A, "Steve", true)));
    assert_eq!(e.find_user(A), Some(known(A, "Steve", true)));
    assert!(e.take_warnings().is_empty(), "nothing is damaged any more");
}

/// [`MemoryBackend`] that remembers how hard each write tried to reach the disk.
#[derive(Default)]
struct Recording {
    inner: MemoryBackend,
    writes: Arc<Mutex<Vec<Durability>>>,
}

impl Backend for Recording {
    fn read(&self, f: &mut dyn FnMut(&dyn ReadOps) -> Result<()>) -> Result<()> {
        self.inner.read(f)
    }

    fn write(&self, durability: Durability, f: &mut dyn FnMut(&mut dyn WriteOps) -> Result<()>) -> Result<()> {
        self.writes.lock().unwrap().push(durability);
        self.inner.write(durability, f)
    }
}

#[test]
fn name_writes_reach_a_durable_commit() {
    let backend = Recording::default();
    let writes = backend.writes.clone();
    let s = PermsStore::new(Store::new(backend)).unwrap();
    writes.lock().unwrap().clear();
    for i in 0..2500 {
        let name = if i % 2 == 0 { "Steve" } else { "Steve2" };
        s.save_name(&known(A, name, false), &mut Vec::new()).unwrap();
    }
    let all = writes.lock().unwrap().clone();
    assert_eq!(all.len(), 2500, "every rename is written");
    let durable = all.iter().filter(|d| **d == Durability::Immediate).count();
    let longest = all.split(|d| *d == Durability::Immediate).map(<[_]>::len).max();
    assert_eq!((durable, longest), (2, Some(999)));
    // The same join again writes nothing.
    s.save_name(&known(A, "Steve2", false), &mut Vec::new()).unwrap();
    assert_eq!(writes.lock().unwrap().len(), 2500);
}
