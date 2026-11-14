//! Writes a database with `N` known players, as PumboPerms 1.0.0 left them:
//! names only in `perms_names` (no `perms_name_keys`), schema 2. The names are
//! `Player0` to `Player<N-1>` with the UUIDs `00000000-0000-4000-8000-<i in 12
//! digits>`. The live memory steps start a server on such a file. With
//! `--ranked` every player is also in the group `vip`, and with `--own-node`
//! has a node of its own as well (`plots.plot.<i>`).
//!
//! `cargo run --release -p pumbo-perms-core --example names_fixture -- <perms.redb> <N> [--ranked [--own-node]]`
//!
//! An existing file at that path is replaced.

use std::path::Path;
use std::process::ExitCode;

use pumbo_common::store::{Store, WriteExt};
use pumbo_perms_core::context::Contexts;
use pumbo_perms_core::model::{Data, Group, ParentNode, PermNode, User};
use pumbo_perms_core::perms::DEFAULT_GROUP;
use pumbo_perms_core::store::{GROUPS, NAMES, PermsStore, USERS};

const USAGE: &str = "usage: names_fixture <perms.redb> <N> [--ranked [--own-node]]";

/// What every player has besides the name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ranks {
    /// A record in the group `vip`.
    pub ranked: bool,
    /// And in it a node of its own, `plots.plot.<i>`.
    pub own_node: bool,
}

/// The UUID of player `i`.
pub fn uuid(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}

/// Writes `n` players to a new file at `path`.
pub fn write(path: &Path, n: u32, ranks: Ranks) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("cannot replace {}: {e}", path.display()));
        }
        _ => {}
    }
    let store = Store::open(path).map_err(|e| e.to_string())?;
    store
        .write(|tx| {
            if ranks.ranked {
                tx.put_json(GROUPS, DEFAULT_GROUP, &Group::new(DEFAULT_GROUP))?;
                tx.put_json(GROUPS, "vip", &Group::new("vip"))?;
            }
            for i in 0..n {
                let (uuid, name) = (uuid(i), format!("Player{i}"));
                tx.put_json(NAMES, &format!("player{i}"), &serde_json::json!({ "uuid": uuid, "name": name }))?;
                if ranks.ranked {
                    let parent = ParentNode { group: "vip".into(), context: Contexts::global(), expiry: None };
                    let mut data = Data { parents: vec![parent], ..Data::default() };
                    if ranks.own_node {
                        let node = format!("plots.plot.{i}");
                        data.permissions.push(PermNode {
                            node,
                            value: true,
                            context: Contexts::global(),
                            expiry: None,
                        });
                    }
                    tx.put_json(USERS, &uuid, &User { uuid: uuid.clone(), name, data })?;
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    PermsStore::new(store).map_err(|e| e.to_string())?;
    Ok(())
}

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(path), Some(n)) = (args.first(), args.get(1)) else { return Err(USAGE.into()) };
    let n: u32 = n.parse().map_err(|e| format!("N must be a number: {e}"))?;
    let mut ranks = Ranks::default();
    for flag in args.iter().skip(2) {
        match flag.as_str() {
            "--ranked" => ranks.ranked = true,
            "--own-node" => ranks.own_node = true,
            _ => return Err(USAGE.into()),
        }
    }
    if ranks.own_node && !ranks.ranked {
        return Err(USAGE.into());
    }
    let started = std::time::Instant::now();
    write(Path::new(path), n, ranks)?;
    let what = match (ranks.ranked, ranks.own_node) {
        (false, _) => "names",
        (true, false) => "players in vip",
        (true, true) => "players in vip with a node each",
    };
    Ok(format!("names_fixture: {n} {what} in {path} ({} ms)", started.elapsed().as_millis()))
}
