//! The plugin on the fake host of the SDK (`pumbo_sdk::testing`).

use pumbo_common::config::{self as common_config};
use pumbo_common::lang::{Lang, check_bundle};
use pumbo_common::store::Store;
use pumbo_perms_core::command::{ACTIONS, split_args};
use pumbo_perms_core::export;
use pumbo_perms_core::ipc;
use pumbo_sdk::bindings::pumbo::prox::servers::ServerInfo;
use pumbo_sdk::contracts::{
    CheckOffline, CheckOfflineAnswer, HOST, METHOD_CHECK_OFFLINE, METHOD_EXPORT, METHOD_FILE, METHOD_INFO, PERMISSIONS,
    PermissionsExport, PermissionsFile, PermissionsInfo, PermissionsInfoRequest,
};
use pumbo_sdk::testing::{self, block_on};
use pumbo_sdk::{CommandEvent, Context, PlaceholderRequest, Plugin};

use super::*;

mod commands;
mod import;
mod provider;
mod scale;
mod service;

const MANIFEST: &str = include_str!("../../pumbo-perms.yml");

fn server(name: &str, groups: &[&str]) -> ServerInfo {
    ServerInfo {
        name: name.into(),
        address: "127.0.0.1:1".into(),
        protocol: None,
        online: true,
        players: 0,
        groups: groups.iter().map(|g| g.to_string()).collect(),
        enforces_secure_chat: None,
    }
}

fn plugin(config: Option<&str>) -> (PumboPerms, std::path::PathBuf) {
    plugin_on(config, PermsStore::in_memory().unwrap())
}

fn plugin_on(config: Option<&str>, store: PermsStore) -> (PumboPerms, std::path::PathBuf) {
    plugin_after(config, store, || {})
}

/// [`plugin_on`] with `before` run on the fresh host, as another plugin loaded earlier.
fn plugin_after(config: Option<&str>, store: PermsStore, before: impl FnOnce()) -> (PumboPerms, std::path::PathBuf) {
    testing::reset();
    testing::with(|h| h.servers = vec![server("lobby", &["hubs"]), server("survival", &[])]);
    before();
    let dir = std::env::temp_dir().join(format!(
        "pumbo-perms-prox-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(dir.join("imports"));
    let _ = std::fs::remove_dir_all(dir.join("exports"));
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::remove_file(dir.join("config.yml"));
    let _ = std::fs::remove_file(dir.join(common_config::LAST_GOOD));
    if let Some(c) = config {
        std::fs::write(dir.join("config.yml"), c).unwrap();
    }
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Ok(store)).unwrap();
    (p, dir)
}

/// [`plugin`] whose engine reads players' records from `store` when they are
/// needed, as it does on a database file.
fn plugin_reading(store: PermsStore) -> PumboPerms {
    let (p, _) = plugin(None);
    p.with(|s| {
        let mut engine = Engine::lazy_on(Ok(store), s.engine.cfg.clone(), s.engine.lang.clone());
        engine.server_groups = std::mem::take(&mut s.engine.server_groups);
        engine.root = s.engine.root.clone();
        s.engine = engine;
    });
    p
}

/// Writes players in `vip` (and the group) to the database as an earlier
/// start left them, and reads it again.
fn in_vip(p: &PumboPerms, uuids: &[String]) {
    use pumbo_perms_core::model::{Data, Group, ParentNode, User};
    let parent =
        ParentNode { group: "vip".into(), context: pumbo_perms_core::context::Contexts::global(), expiry: None };
    let users = uuids
        .iter()
        .map(|u| User {
            uuid: u.clone(),
            name: String::new(),
            data: Data { parents: vec![parent.clone()], ..Data::default() },
        })
        .collect();
    let (perms, mut dirty) =
        pumbo_perms_core::perms::Perms::from_parts(vec![Group::new("default"), Group::new("vip")], users, Vec::new());
    dirty.groups.extend(perms.groups().map(|g| g.name.clone()));
    dirty.users.extend(perms.users().map(|u| u.uuid.clone()));
    p.with(|s| {
        s.engine.store().unwrap().commit(&perms, &dirty, None, 0).unwrap();
        s.engine.reload_data();
    })
    .unwrap();
}

fn console(p: &PumboPerms, line: &str) {
    block_on(p.run(None, split_args(line)));
}

fn load(p: &PumboPerms, id: PlayerId) -> Vec<PermissionEntry> {
    block_on(p.on_permission_load(players::get(id).unwrap())).unwrap().entries
}

fn has(set: &[PermissionEntry], node: &str, ctx: Context, value: bool) -> bool {
    set.iter().any(|e| e.node == node && e.context == ctx && e.value == value)
}

fn last_replace(id: PlayerId) -> Option<Vec<PermissionEntry>> {
    testing::with(|h| h.replaced.iter().rev().find(|(p, _)| *p == id).map(|(_, s)| s.entries.clone()))
}

fn logs() -> String {
    testing::with(|h| h.logs.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>().join("\n"))
}

fn file(json: &str) -> Vec<u8> {
    let digest = format!("{:x}", json.len() * 31 + json.bytes().map(usize::from).sum::<usize>());
    cbor(&PermissionsFile { fingerprint: digest, data: json.to_string() }).unwrap()
}

/// A service call from the proxy host.
fn call(p: &PumboPerms, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, CallReject> {
    call_as(p, HOST, method, payload)
}

fn call_as(p: &PumboPerms, caller: &str, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, CallReject> {
    let mut c = testing::service_call(PERMISSIONS.name, 1, method, payload);
    c.caller = caller.into();
    block_on(p.on_service_call(c))
}

fn registered() -> Vec<String> {
    testing::with(|h| h.commands.iter().filter(|c| !c.umbrella).map(|c| c.name.clone()).collect())
}

fn command(p: &PumboPerms, name: &str, args: &[&str]) {
    let e = CommandEvent { player: None, name: name.into(), args: args.iter().map(|a| a.to_string()).collect() };
    block_on(p.on_command(e));
}

fn contracts_request() -> pumbo_sdk::contracts::PermissionsExportRequest {
    pumbo_sdk::contracts::PermissionsExportRequest { server: "lobby".into(), groups: vec!["hubs".into()] }
}
