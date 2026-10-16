//! Heap use with many known players, counted by this test's global
//! allocator. Release benchmarks, one at a time:
//! `cargo test --release -p pumbo-perms-core --test memory -- --ignored --nocapture`.

#![allow(unsafe_code, clippy::unwrap_used, clippy::print_stdout)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use pumbo_common::lang::{COMMON, Lang};
use pumbo_common::store::{Store, WriteExt};
use pumbo_perms_core::command::{self, Env, Who, split_args};
use pumbo_perms_core::config::CoreCfg;
use pumbo_perms_core::context::Contexts;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::model::ParentNode;
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::store::{NAMES, PermsStore};

/// The system allocator, counting the bytes in use and their peak.
struct Counting;

static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(bytes: usize) {
    let now = IN_USE.fetch_add(bytes, Ordering::Relaxed).saturating_add(bytes);
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: every call goes to `System` with the caller's arguments; the
// counters only read the sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is passed on unchanged.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as in `alloc`.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as in `alloc`.
        unsafe { System.dealloc(ptr, layout) };
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as in `alloc`.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
            grew(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The counters are global: the tests measure one after another.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Starts counting the peak again; returns the bytes in use now.
fn start() -> usize {
    let now = IN_USE.load(Ordering::Relaxed);
    PEAK.store(now, Ordering::Relaxed);
    now
}

/// How far the heap went above `base` since [`start`], in MiB.
fn peak_mib(base: usize) -> f64 {
    PEAK.load(Ordering::Relaxed).saturating_sub(base) as f64 / (1024.0 * 1024.0)
}

/// How far the heap may grow with any number of known names: the page
/// cache of redb is 16 MiB of it.
const NAMES_BUDGET_MIB: f64 = 24.0;

/// With 20 000 players in a group: their data takes about 11 MiB here (8-byte
/// pointers; less in WebAssembly) and a change copies it once to be able to
/// undo it, next to the page cache of redb.
const CHANGE_BUDGET_MIB: f64 = 32.0;

fn uuid(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

/// A fresh database file in a folder of its own.
fn database(test: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-memory-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("perms.redb");
    (dir, path)
}

/// `Player0`.. as earlier builds wrote them: only `perms_names`.
fn write_names(path: &Path, n: u32) {
    let store = Store::open(path).unwrap();
    store
        .write(|tx| {
            for i in 0..n {
                let record = serde_json::json!({ "uuid": uuid(i), "name": format!("Player{i}") });
                tx.put_json(NAMES, &format!("player{i}"), &record)?;
            }
            Ok(())
        })
        .unwrap();
    PermsStore::new(store).unwrap();
}

fn engine(path: &Path, lang: Lang) -> Engine {
    Engine::new(PermsStore::new(Store::open(path).unwrap()), CoreCfg::default(), lang)
}

fn lang() -> Lang {
    Lang::load(&[COMMON, pumbo_perms_core::LANG], "en", None).0
}

/// 400 000 known names: the engine keeps none of them in memory.
#[test]
#[ignore = "release benchmark"]
fn known_names_memory_stays_flat() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let (dir, path) = database("names");
    write_names(&path, 400_000);
    let lang = lang();
    let base = start();
    let mut e = engine(&path, lang);
    for i in 0..200u32 {
        // New players, and known ones under a new name.
        let (id, name) = match i % 2 {
            0 => (format!("00000000-0000-4000-9000-{i:012}"), format!("Newcomer{i}")),
            _ => (uuid(i), format!("Renamed{i}")),
        };
        e.seen(&id, &name, true).unwrap();
    }
    assert_eq!(e.find_user("Player399999").map(|k| k.uuid), Some(uuid(399_999)));
    assert_eq!(e.find_user("renamed1").map(|k| k.uuid), Some(uuid(1)));
    assert!(e.find_user("Player1").is_none());
    assert_eq!(e.find_user(&uuid(399_998)).map(|k| k.uuid), Some(uuid(399_998)));
    let grown = peak_mib(base);
    println!("memory: 400000 names, heap peak +{grown:.1} MiB");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(grown <= NAMES_BUDGET_MIB, "{grown:.1} MiB");
}

/// 20 000 players in one group and one `/pp` change: a change copies the
/// groups and the players with data, so that it can be undone.
#[test]
#[ignore = "release benchmark"]
fn change_among_20000_players_fits_the_budget() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let (dir, path) = database("change");
    write_names(&path, 20_000);
    {
        let mut e = engine(&path, lang());
        let mut dirty = e.perms.create_group("vip").unwrap();
        for i in 0..20_000 {
            let parent = ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None };
            dirty.merge(e.perms.edit_user(&uuid(i), &format!("Player{i}"), |d| d.add_parent(parent)).1);
        }
        e.commit(&dirty, None).unwrap();
    }
    let lang = lang();
    let base = start();
    let mut e = engine(&path, lang);
    let none = |_: &str| -> Option<KnownName> { None };
    let env = Env { now: 1, online: &none, version: "0", version_rows: Vec::new() };
    let out = command::run(&mut e, &Who::console(), &split_args("user Player19999 permission set fly"), &env);
    assert!(out.changed, "{}", out.reply.plain());
    let grown = peak_mib(base);
    println!("memory: 20000 players in a group, one change, heap peak +{grown:.1} MiB");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(grown <= CHANGE_BUDGET_MIB, "{grown:.1} MiB");
}
