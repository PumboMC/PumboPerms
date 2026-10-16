//! Writes a database with `N` known players, as PumboPerms 1.0.0 left them:
//! names only in `perms_names` (no `perms_name_keys`), schema 2. The names are
//! `Player0` to `Player<N-1>` with the UUIDs `00000000-0000-4000-8000-<i in 12
//! digits>`. The live memory steps start a server on such a file.
//!
//! `cargo run --release -p pumbo-perms-core --example names_fixture -- <perms.redb> <N>`
//!
//! An existing file at that path is replaced.

use std::process::ExitCode;

use pumbo_common::store::{Store, WriteExt};
use pumbo_perms_core::store::{NAMES, PermsStore};

fn main() -> ExitCode {
    match run() {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("names_fixture: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<String, String> {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(n), None) = (args.next(), args.next(), args.next()) else {
        return Err("usage: names_fixture <perms.redb> <N>".into());
    };
    let n: u32 = n.parse().map_err(|e| format!("N must be a number: {e}"))?;
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("cannot replace {path}: {e}")),
        _ => {}
    }
    let started = std::time::Instant::now();
    let store = Store::open(&path).map_err(|e| e.to_string())?;
    store
        .write(|tx| {
            for i in 0..n {
                let record =
                    serde_json::json!({ "uuid": format!("00000000-0000-4000-8000-{i:012}"), "name": format!("Player{i}") });
                tx.put_json(NAMES, &format!("player{i}"), &record)?;
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    PermsStore::new(store).map_err(|e| e.to_string())?;
    Ok(format!("names_fixture: {n} names in {path} ({} ms)", started.elapsed().as_millis()))
}
