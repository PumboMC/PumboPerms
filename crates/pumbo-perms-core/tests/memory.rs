//! Heap use with many known players, counted by this test's global
//! allocator. Release benchmarks, one at a time:
//! `cargo test --release -p pumbo-perms-core --test memory -- --ignored --nocapture`.

#![allow(unsafe_code, clippy::unwrap_used, clippy::print_stdout)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use pumbo_common::lang::{COMMON, Lang};
use pumbo_common::store::Store;
use pumbo_perms_core::command::{self, Env, Outcome, SourceMode, Who, split_args};
use pumbo_perms_core::config::CoreCfg;
use pumbo_perms_core::context::Contexts;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::model::{Data, ParentNode, User};
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::store::PermsStore;

#[path = "../examples/names_fixture.rs"]
#[allow(dead_code, reason = "its `main` runs only as the example")]
mod names_fixture;

use names_fixture::{Ranks, uuid};

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

fn one_at_a_time() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts counting the peak again; returns the bytes in use now.
fn start() -> usize {
    let now = IN_USE.load(Ordering::Relaxed);
    PEAK.store(now, Ordering::Relaxed);
    now
}

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// How far the heap went above `base` since [`start`], in MiB.
fn peak_mib(base: usize) -> f64 {
    mib(PEAK.load(Ordering::Relaxed).saturating_sub(base))
}

/// How far the heap is above `base` now, in MiB.
fn held_mib(base: usize) -> f64 {
    mib(IN_USE.load(Ordering::Relaxed).saturating_sub(base))
}

/// How far the heap may grow with any number of known names: the page
/// cache of redb is 16 MiB of it.
const NAMES_BUDGET_MIB: f64 = 24.0;

/// With 20 000 players in a group: their data takes about 11 MiB here (8-byte
/// pointers; less in WebAssembly) and a change copies it once to be able to
/// undo it, next to the page cache of redb.
const CHANGE_BUDGET_MIB: f64 = 32.0;

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
    names_fixture::write(path, n, Ranks::default()).unwrap();
}

/// `Player0`.. each in the group `vip`.
fn write_ranked(path: &Path, n: u32) {
    names_fixture::write(path, n, Ranks { ranked: true, own_node: false }).unwrap();
}

fn engine(path: &Path, lang: Lang) -> Engine {
    Engine::new(PermsStore::new(Store::open(path).unwrap()), CoreCfg::default(), lang)
}

fn lang() -> Lang {
    Lang::load(&[COMMON, pumbo_perms_core::LANG], "en", None).0
}

/// `/pp <line>` from the console at `now`.
fn run(e: &mut Engine, line: &str, now: u64) -> Outcome {
    let none = |_: &str| -> Option<KnownName> { None };
    let env = Env { now, online: &none, version: "0", version_rows: Vec::new() };
    command::run(e, &Who::console(), &split_args(line), &env)
}

#[test]
fn ranked_fixture_gives_every_player_a_group() {
    let (dir, path) = database("fixture");
    names_fixture::write(&path, 50, Ranks { ranked: true, own_node: true }).unwrap();
    let (perms, _) = PermsStore::new(Store::open(&path).unwrap()).unwrap().load().unwrap();
    assert_eq!(perms.users().count(), 50);
    for i in 0..50 {
        let data = &perms.user(&uuid(i)).unwrap().data;
        assert_eq!(data.parents.iter().map(|p| p.group.as_str()).collect::<Vec<_>>(), ["vip"]);
        assert_eq!(data.permissions.iter().map(|p| p.node.clone()).collect::<Vec<_>>(), [format!("plots.plot.{i}")]);
    }
    assert!(perms.group("vip").is_some() && perms.group("default").is_some());
    names_fixture::write(&path, 50, Ranks::default()).unwrap();
    let (perms, dirty) = PermsStore::new(Store::open(&path).unwrap()).unwrap().load().unwrap();
    assert!(perms.users().next().is_none() && dirty.groups.contains("default"), "names only");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 400 000 known names: the engine keeps none of them in memory.
#[test]
#[ignore = "release benchmark"]
fn known_names_memory_stays_flat() {
    let _one = one_at_a_time();
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
        e.seen(&id, &name, true, 0).unwrap();
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
    let _one = one_at_a_time();
    let (dir, path) = database("change");
    write_ranked(&path, 20_000);
    let lang = lang();
    let base = start();
    let mut e = engine(&path, lang);
    let out = run(&mut e, "user Player19999 permission set fly", 1);
    assert!(out.changed, "{}", out.reply.plain());
    let grown = peak_mib(base);
    println!("memory: 20000 players in a group, one change, heap peak +{grown:.1} MiB");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(grown <= CHANGE_BUDGET_MIB, "{grown:.1} MiB");
}

/// One change of a player and one of their group among 20 000 and 100 000
/// players in the group, each counted from the state before it: a change
/// keeps only the records it touches, so it costs the same at both sizes
/// (what is left is the write transaction of redb).
#[test]
#[ignore = "release benchmark"]
fn one_change_copies_what_it_touches() {
    let _one = one_at_a_time();
    let mut peaks = Vec::new();
    for n in [20_000u32, 100_000] {
        let (dir, path) = database(&format!("touch-{n}"));
        write_ranked(&path, n);
        let mut e = engine(&path, lang());
        let base = start();
        let started = Instant::now();
        assert!(run(&mut e, &format!("user Player{} permission set fly", n - 1), 1).changed);
        let (player, player_took) = (peak_mib(base), started.elapsed());
        let base = start();
        let started = Instant::now();
        assert!(run(&mut e, "group vip permission set fly", 1).changed);
        let (group, group_took) = (peak_mib(base), started.elapsed());
        println!(
            "memory: {n} players in a group, one change of a player +{player:.2} MiB ({player_took:?}), of the group +{group:.2} MiB ({group_took:?})"
        );
        peaks.push((player, group));
        drop(e);
        let _ = std::fs::remove_dir_all(&dir);
    }
    let (small, large) = (peaks[0], peaks[1]);
    for (small, large) in [(small.0, large.0), (small.1, large.1)] {
        assert!(small < 2.0 && large < 2.0 && (large - small).abs() <= 0.25, "{peaks:?}");
    }
}

/// A player joins for whom an import waits, among 20 000 players in a group.
#[test]
#[ignore = "release benchmark"]
fn pending_import_at_join_copies_nothing() {
    let _one = one_at_a_time();
    let (dir, path) = database("pending");
    write_ranked(&path, 20_000);
    let mut e = engine(&path, lang());
    let parent = ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None };
    let waiting =
        User { uuid: String::new(), name: "Newcomer".into(), data: Data { parents: vec![parent], ..Data::default() } };
    let snap = Snapshot {
        format: export::FORMAT.into(),
        version: export::VERSION,
        by_name: vec![waiting],
        ..Snapshot::default()
    };
    command::import_source(&mut e, &Who::console(), "file", Ok(("f1".into(), snap)), SourceMode::Run, 1);
    assert_eq!(e.ledger("file").unwrap().pending.len(), 1);
    let newcomer = "00000000-0000-4000-9000-000000000001";
    e.seen(newcomer, "Newcomer", true, 2).unwrap();
    let base = start();
    let joined = command::on_join(&mut e, newcomer, "Newcomer", true, 2);
    let grown = peak_mib(base);
    assert!(joined.iter().any(|o| o.changed), "{:?}", joined.iter().map(|o| o.reply.plain()).collect::<Vec<_>>());
    println!("memory: 20000 players in a group, a join with an import waiting for it, heap peak +{grown:.2} MiB");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 100 000 and 400 000 players in a group, counted from before the start: the
/// start, then 200 joins and leaves and 50 commands on players who are not
/// online. Each player with a node of its own: printed only.
#[test]
#[ignore = "release benchmark"]
fn offline_players_stay_on_disk() {
    let _one = one_at_a_time();
    let own = Ranks { ranked: true, own_node: true };
    for (n, ranks) in
        [(100_000u32, Ranks { own_node: false, ..own }), (400_000, Ranks { own_node: false, ..own }), (100_000, own)]
    {
        let (dir, path) = database(&format!("offline-{n}"));
        names_fixture::write(&path, n, ranks).unwrap();
        let lang = lang();
        let base = start();
        let started = Instant::now();
        let mut e = engine(&path, lang);
        let (opened, open_peak, open_held) = (started.elapsed(), peak_mib(base), held_mib(base));
        for i in 0..200u32 {
            let (id, name) = (uuid(i), format!("Player{i}"));
            e.seen(&id, &name, true, 1).unwrap();
            assert_eq!(e.decide(&id, "group.vip", "", 0, 1), Some(true));
            e.gone(&id);
        }
        for i in 0..50u32 {
            assert!(run(&mut e, &format!("user Player{} permission set extra.node", n - 1 - i), 1).changed);
        }
        let (peak, held) = (peak_mib(base), held_mib(base));
        let what = if ranks.own_node { "players in a group with a node each" } else { "players in a group" };
        println!(
            "memory: {n} {what}: start {opened:?}, peak +{open_peak:.1} MiB, held +{open_held:.1} MiB; after 200 joins and 50 commands peak +{peak:.1} MiB, held +{held:.1} MiB"
        );
        drop(e);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `/pp export` of 100 000 players in a group, counted from the state after the start.
#[test]
#[ignore = "release benchmark"]
fn export_of_100000_players_stays_flat() {
    let _one = one_at_a_time();
    let (dir, path) = database("export");
    write_ranked(&path, 100_000);
    let e = engine(&path, lang());
    let base = start();
    let started = Instant::now();
    let file = pumbo_perms_core::files::export_to(&e, &dir, "all", 1).unwrap();
    let (grown, took) = (peak_mib(base), started.elapsed());
    drop(e);
    let snap = export::parse(&std::fs::read_to_string(dir.join(file)).unwrap()).unwrap();
    assert_eq!(snap.users.len(), 100_000);
    println!("memory: export of 100000 players in a group, heap peak +{grown:.1} MiB ({took:?})");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `/pp group vip rename vip2` with 100 000 players in `vip`.
#[test]
#[ignore = "release benchmark"]
fn group_rename_among_100000_players() {
    let _one = one_at_a_time();
    let (dir, path) = database("rename");
    write_ranked(&path, 100_000);
    let mut e = engine(&path, lang());
    let base = start();
    let started = Instant::now();
    let out = run(&mut e, "group vip rename vip2", 1);
    let (grown, took) = (peak_mib(base), started.elapsed());
    assert!(out.changed, "{}", out.reply.plain());
    println!("memory: rename of a group with 100000 players, heap peak +{grown:.1} MiB ({took:?})");
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}
