//! Sets for the proxy: what each player gets, and handing them over.

use std::collections::{HashMap, HashSet};

use pumbo_common::clock::now_ms;
use pumbo_perms_core::context::Query;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::ipc;
use pumbo_sdk::{Context, PermissionEntry, PermissionSet, PlayerId, log, permissions, players, scheduler};

use crate::setup;
use crate::{PORTION, PumboPerms, State, server_groups};

/// [`setup::provider_set`] as permission entries for the proxy.
pub(super) fn entries(e: &mut Engine, uuid: &str, now: u64) -> Vec<PermissionEntry> {
    setup::provider_set(e, uuid, now)
        .into_iter()
        .map(|(server, node, value)| PermissionEntry {
            node,
            value,
            context: match server {
                Some(s) => Context::Server(s),
                None => Context::Global,
            },
        })
        .collect()
}

/// [`ipc::info`] on `server` (empty: global) as JSON, built as the host's
/// `info` call builds it: the server and its groups, no fallback context.
pub(super) fn info_json(e: &mut Engine, uuid: &str, server: &str, now: u64) -> String {
    let groups = e.server_groups.get(&server.to_lowercase()).cloned().unwrap_or_default();
    let q = Query::at(server, groups, "");
    ipc::info(e, uuid, &q, now).and_then(|i| serde_json::to_string(&i).ok()).unwrap_or_default()
}

impl PumboPerms {
    /// Sends the proxy the new set of every loaded player a change since the
    /// last call can affect (everyone after the server list changed) whose
    /// set or info differs (a new set also makes the proxy ask for the info
    /// again). The work goes in portions with a pause between them, so a
    /// change that touches every player stays inside the plugin time budget.
    pub(super) async fn push_all(&self) {
        let groups = server_groups();
        let at: HashMap<PlayerId, String> =
            players::all().into_iter().map(|p| (p.id, p.context.server.unwrap_or_default())).collect();
        let todo = self
            .with(|s| {
                let moved = s.engine.server_groups != groups;
                s.engine.server_groups = groups;
                let State { engine, loaded, .. } = s;
                let affected: HashSet<&str> =
                    engine.take_affected(loaded.values().map(|l| l.0.as_str())).into_iter().collect();
                let ids = loaded.iter().filter(|(_, l)| moved || affected.contains(l.0.as_str())).map(|(id, _)| *id);
                ids.collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for portion in todo.chunks(PORTION) {
            // Also after the change itself: an import of a large file is a long piece of work.
            scheduler::sleep(0).await;
            let now = now_ms();
            let changes = self
                .with(|s| {
                    let State { engine, loaded, .. } = s;
                    let mut out = Vec::new();
                    for id in portion {
                        let Some((uuid, last, last_info)) = loaded.get(id) else { continue };
                        let set = entries(engine, uuid, now);
                        let info = info_json(engine, uuid, at.get(id).map_or("", String::as_str), now);
                        if *last != set || *last_info != info {
                            out.push((*id, set, info));
                        }
                    }
                    out
                })
                .unwrap_or_default();
            self.send_sets(changes);
        }
    }

    /// Hands new sets to the proxy and remembers what it took.
    fn send_sets(&self, changes: Vec<(PlayerId, Vec<PermissionEntry>, String)>) {
        for (id, set, info) in changes {
            if pumbo_sdk::debug_enabled() {
                log::debug(&format!("PumboPerms: player {id}: {} entries", set.len()));
            }
            let sent = PermissionSet { entries: set };
            let result = permissions::replace(id, &sent);
            if result.is_ok() {
                // Kept only once the proxy has it: a refused set is sent again next time.
                self.with(|s| {
                    if let Some(l) = s.loaded.get_mut(&id) {
                        (l.1, l.2) = (sent.entries, info);
                    }
                });
            }
            if let Err(e) = result {
                let first = self.with(|s| !std::mem::replace(&mut s.not_provider, true)).unwrap_or(false);
                if first {
                    log::warn(&format!(
                        "PumboPerms: the proxy did not take the permissions ({e}); set permissions.provider to auto or pumbo-perms in pumboprox.yml"
                    ));
                }
            }
        }
    }
}
