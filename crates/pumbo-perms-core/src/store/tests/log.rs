//! `/pp log` reads one page from the newest end, and the trim stops at the
//! oldest entries.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pumbo_common::store::{Backend, Durability, MemoryBackend, ReadOps, Result, Store, WriteOps};

use super::entry;
use crate::perms::{Dirty, Perms};
use crate::store::{LOG, LogEntry, PermsStore, append_log};

/// What the log table handed out.
#[derive(Debug, Default)]
struct Reads {
    /// Entries visited by `scan`, `scan_rev` and `retain`.
    entries: AtomicU64,
    /// Forward scans (`collect_prefix` is one).
    forward: AtomicU64,
}

impl Reads {
    /// The counts so far, starting again from zero.
    fn take(&self) -> (u64, u64) {
        (self.entries.swap(0, Ordering::SeqCst), self.forward.swap(0, Ordering::SeqCst))
    }
}

/// [`MemoryBackend`] that counts what the log table hands out.
struct Counting(MemoryBackend, Arc<Reads>);

impl Backend for Counting {
    fn read(&self, f: &mut dyn FnMut(&dyn ReadOps) -> Result<()>) -> Result<()> {
        self.0.read(&mut |tx| f(&Read(tx, &self.1)))
    }

    fn write(&self, durability: Durability, f: &mut dyn FnMut(&mut dyn WriteOps) -> Result<()>) -> Result<()> {
        self.0.write(durability, &mut |tx| f(&mut Write(tx, &self.1)))
    }
}

struct Read<'a>(&'a dyn ReadOps, &'a Reads);

struct Write<'a>(&'a mut dyn WriteOps, &'a Reads);

/// `visit`, counting the log entries it gets.
fn counted<'a>(
    reads: &'a Reads,
    table: &str,
    visit: &'a mut dyn FnMut(&str, &[u8]) -> bool,
) -> impl FnMut(&str, &[u8]) -> bool + 'a {
    let log = table == LOG;
    move |k, v| {
        if log {
            reads.entries.fetch_add(1, Ordering::SeqCst);
        }
        visit(k, v)
    }
}

impl ReadOps for Read<'_> {
    fn get(&self, table: &str, key: &str) -> Result<Option<Vec<u8>>> {
        self.0.get(table, key)
    }

    fn scan(&self, table: &str, prefix: &str, visit: &mut dyn FnMut(&str, &[u8]) -> bool) -> Result<()> {
        if table == LOG {
            self.1.forward.fetch_add(1, Ordering::SeqCst);
        }
        self.0.scan(table, prefix, &mut counted(self.1, table, visit))
    }

    fn scan_rev(&self, table: &str, visit: &mut dyn FnMut(&str, &[u8]) -> bool) -> Result<()> {
        self.0.scan_rev(table, &mut counted(self.1, table, visit))
    }

    fn len(&self, table: &str) -> Result<u64> {
        self.0.len(table)
    }
}

impl ReadOps for Write<'_> {
    fn get(&self, table: &str, key: &str) -> Result<Option<Vec<u8>>> {
        self.0.get(table, key)
    }

    fn scan(&self, table: &str, prefix: &str, visit: &mut dyn FnMut(&str, &[u8]) -> bool) -> Result<()> {
        Read(&*self.0, self.1).scan(table, prefix, visit)
    }

    fn scan_rev(&self, table: &str, visit: &mut dyn FnMut(&str, &[u8]) -> bool) -> Result<()> {
        Read(&*self.0, self.1).scan_rev(table, visit)
    }

    fn len(&self, table: &str) -> Result<u64> {
        self.0.len(table)
    }
}

impl WriteOps for Write<'_> {
    fn put(&mut self, table: &str, key: &str, value: &[u8]) -> Result<()> {
        self.0.put(table, key, value)
    }

    fn remove(&mut self, table: &str, key: &str) -> Result<bool> {
        self.0.remove(table, key)
    }

    fn retain(&mut self, table: &str, keep: &mut dyn FnMut(&str, &[u8]) -> bool) -> Result<u64> {
        self.0.retain(table, &mut counted(self.1, table, keep))
    }
}

/// A log of `n` entries and its counters at zero. The entries go in one
/// transaction: the memory backend copies every table on each write.
fn log_of(n: u64) -> (PermsStore, Arc<Reads>) {
    let reads = Arc::new(Reads::default());
    let s = PermsStore::new(Store::new(Counting(MemoryBackend::default(), Arc::clone(&reads)))).unwrap();
    s.store.write(|tx| (1..=n).try_for_each(|seq| append_log(tx, &entry(seq), 0))).unwrap();
    reads.take();
    (s, reads)
}

fn times(entries: &[LogEntry]) -> Vec<u64> {
    entries.iter().map(|e| e.time).collect()
}

#[test]
fn log_page_reads_one_page_from_the_end() {
    let (s, reads) = log_of(100_000);
    let (page, total) = s.log_page(1, 10).unwrap();
    assert_eq!(times(&page), (99_991..=100_000).rev().collect::<Vec<u64>>());
    assert_eq!(total, 100_000);
    let (entries, forward) = reads.take();
    assert!(entries <= 10 && forward == 0, "{entries} entries, {forward} forward scans");

    let (page, _) = s.log_page(3, 10).unwrap();
    assert_eq!(times(&page), (99_971..=99_980).rev().collect::<Vec<u64>>());
    let (entries, forward) = reads.take();
    assert!(entries <= 30 && forward == 0, "{entries} entries, {forward} forward scans");
}

#[test]
fn log_page_past_the_end_reads_nothing() {
    let (s, reads) = log_of(30);
    // the last page is not past the end
    assert_eq!(times(&s.log_page(3, 10).unwrap().0), (1..=10).rev().collect::<Vec<u64>>());
    reads.take();
    for (page, per_page) in [(4, 10), (99, 10), (usize::MAX, 10), (1, 0)] {
        assert_eq!(s.log_page(page, per_page).unwrap(), (Vec::new(), 30), "page {page} of {per_page}");
    }
    assert_eq!(reads.take(), (0, 0));
}

#[test]
fn log_trim_removes_only_the_oldest() {
    let (s, reads) = log_of(100_000);
    // 100 001 entries are over 90 000 and its tenth: the 10 001 oldest go
    s.commit(&Perms::new(), &Dirty::default(), Some(&entry(100_001)), 90_000).unwrap();
    let (entries, _) = reads.take();
    assert!(entries <= 10_001 + 1, "{entries} entries read to remove 10 001");
    assert_eq!(s.log_page(1, 1).unwrap(), (vec![entry(100_001)], 90_000));
    assert_eq!(times(&s.log_page(90_000, 1).unwrap().0), [10_002]);
}
