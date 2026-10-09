//! The plugin on the fake host of the SDK (`pumbo_sdk::testing`).

use pumbo_common::config::{self as common_config};
use pumbo_common::lang::{Lang, check_bundle};
use pumbo_perms_core::command::ACTIONS;
use pumbo_sdk::Plugin;
use pumbo_sdk::bindings::pumbo::prox::servers::ServerInfo;
use pumbo_sdk::testing::{self, block_on};

use super::*;

const MANIFEST: &str = include_str!("../pumbo-perms.yml");

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
    testing::reset();
    testing::with(|h| h.servers = vec![server("lobby", &["hubs"]), server("survival", &[])]);
    let dir = std::env::temp_dir().join(format!(
        "pumbo-perms-prox-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::create_dir_all(dir.join("imports"));
    let _ = std::fs::remove_file(dir.join("imports/file.json"));
    let _ = std::fs::remove_file(dir.join("config.yml"));
    if let Some(c) = config {
        std::fs::write(dir.join("config.yml"), c).unwrap();
    }
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Ok(PermsStore::in_memory().unwrap())).unwrap();
    (p, dir)
}

fn console(p: &PumboPerms, line: &str) {
    p.run(None, split_args(line));
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

fn call(p: &PumboPerms, method: &str, payload: Vec<u8>) -> Result<Vec<u8>, CallReject> {
    block_on(p.on_service_call(testing::service_call(PERMISSIONS.name, 1, method, payload)))
}

#[test]
fn template_messages_and_manifest() {
    let (cfg, w) = common_config::load::<setup::Config>(setup::CONFIG_TEMPLATE);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg, setup::Config::default());
    assert_eq!(check_bundle(&setup::LANG), Vec::<String>::new());
    for code in ["en", "pl"] {
        let (l, w) = Lang::load(&setup::BUNDLES, code, None);
        assert!(w.is_empty(), "{w:?}");
        assert!(l.get("prefix").contains("PumboPerms"));
    }
    // Every permission is declared with a description.
    let declared: Vec<&str> = MANIFEST.lines().filter_map(|l| l.trim().strip_prefix("- node: ")).collect();
    for a in ACTIONS.iter().copied().chain(["command", "debug"]) {
        assert!(declared.contains(&format!("pumbo.perms.{a}").as_str()), "{a} missing from the manifest");
    }
    assert_eq!(MANIFEST.matches("description:").count(), declared.len());
    // A bad value falls back with a warning; a broken file is refused on reload.
    let (cfg, w) = common_config::load::<setup::Config>("language: \"bad lang!\"\nimport:\n  folow: true\n");
    assert_eq!(cfg.language, "en");
    assert_eq!(w.len(), 2, "{w:?}");
    // Not YAML: the error names the line, /pp reload keeps the settings.
    let (p, dir) = plugin(Some("language: pl\n"));
    std::fs::write(dir.join("config.yml"), "language: [pl\nlog: {}\n").unwrap();
    let err = p.reload().unwrap_err();
    assert!(err.contains("line"), "{err}");
    assert_eq!(p.with(|s| s.cfg.language.clone()).unwrap(), "pl");
    let _ = std::fs::remove_file(dir.join("config.yml"));
}

/// The proxy's permissions.yml comes in once at start: groups, contexts,
/// players by UUID now and by nickname at their first join; the same file
/// again does nothing, a changed one only gives a hint (`follow: false`).
#[test]
fn takes_over_permissions_yml() {
    let (p, dir) = plugin(None);
    let alice = testing::add_player("Alice", Some("lobby"));
    let alice_uuid = uuid_of(&players::get(alice).unwrap());
    let data = format!(
        r#"{{"format":"pumboperms","version":1,
        "groups":[{{"name":"vip","data":{{"permissions":[{{"node":"minecraft:command.gamemode","value":true,"context":{{"server":"lobby"}}}}],
                     "parents":[{{"group":"default"}}]}}}},
                  {{"name":"default","data":{{"permissions":[{{"node":"pumbo.proxy.server","value":true}}]}}}}],
        "users":[{{"uuid":"{alice_uuid}","name":"Alice","data":{{"parents":[{{"group":"vip"}}]}}}}],
        "by-name":[{{"name":"Jeb_","data":{{"parents":[{{"group":"vip"}}]}}}}]}}"#
    );
    call(&p, METHOD_FILE, file(&data)).unwrap();
    assert!(logs().contains("permissions.yml: PumboPerms » Imported file"), "{}", logs());
    assert!(logs().contains("Jeb_"), "pending player named in the log: {}", logs());
    let set = load(&p, alice);
    assert!(has(&set, "minecraft:command.gamemode", Context::Server("lobby".into()), true), "{set:?}");
    assert!(
        !set.iter().any(|e| e.node == "minecraft:command.gamemode" && e.context != Context::Server("lobby".into()))
    );
    assert!(has(&set, "pumbo.proxy.server", Context::Global, true));
    assert!(has(&set, "group.vip", Context::Global, true));
    assert!(std::fs::read_to_string(dir.join("imports/file.json")).unwrap().contains("Jeb_"));

    // Jeb_ joins for the first time: the entries waiting for the name.
    let jeb = testing::add_player("jeb_", Some("survival"));
    let set = load(&p, jeb);
    assert!(has(&set, "group.vip", Context::Global, true), "{set:?}");
    assert!(logs().contains("joined for the first time"), "{}", logs());
    assert!(!std::fs::read_to_string(dir.join("imports/file.json")).unwrap().contains("\"pending\""));

    // The same file at the next start: nothing; a changed one: a hint only.
    let before = logs().len();
    call(&p, METHOD_FILE, file(&data)).unwrap();
    assert_eq!(logs().len(), before);
    call(&p, METHOD_FILE, file(&data.replace("pumbo.proxy.server", "pumbo.proxy.glist"))).unwrap();
    assert!(logs().contains("changed since it was imported"), "{}", logs());
    // By hand it goes in, and undo takes back exactly the import.
    console(&p, "import run file");
    assert!(logs().contains("Imported file"), "{}", logs());
    console(&p, "import undo file");
    let set = last_replace(alice).unwrap();
    assert!(!has(&set, "group.vip", Context::Global, true), "undo took Alice out of vip: {set:?}");
}

/// A change by `/pp` reaches the proxy at once (`replace`), per server
/// context; taking it back too. Offline checks and the export for the
/// servers behind the proxy.
#[test]
fn changes_reach_the_proxy_per_server() {
    let (p, _) = plugin(None);
    let bob = testing::add_player("Bob", Some("survival"));
    let first = load(&p, bob);
    assert!(has(&first, "pumbo.perms.command", Context::Global, false));
    console(&p, "creategroup vip");
    console(&p, "group vip permission set minecraft:command.gamemode");
    console(&p, "user Bob parent add vip server=lobby");
    let set = last_replace(bob).expect("pushed after the change");
    assert!(has(&set, "minecraft:command.gamemode", Context::Server("lobby".into()), true), "{set:?}");
    assert!(!has(&set, "minecraft:command.gamemode", Context::Global, true), "{set:?}");
    assert!(!set.iter().any(|e| e.context == Context::Server("survival".into())), "survival = global: {set:?}");
    let n = testing::with(|h| h.replaced.len());
    console(&p, "group vip info");
    assert_eq!(testing::with(|h| h.replaced.len()), n, "nothing changed, nothing sent");
    console(&p, "user Bob parent remove vip server=lobby");
    let set = last_replace(bob).unwrap();
    assert!(!set.iter().any(|e| e.node == "minecraft:command.gamemode"), "{set:?}");

    // group= contexts follow the proxy's server groups.
    console(&p, "group default permission set pumbo.proxy.glist group=hubs");
    let set = last_replace(bob).unwrap();
    assert!(has(&set, "pumbo.proxy.glist", Context::Server("lobby".into()), true), "{set:?}");

    // Rights for /pp itself: a player with user.info sees /pp.
    console(&p, "user Bob permission set pumbo.perms.user.info");
    let set = last_replace(bob).unwrap();
    assert!(has(&set, "pumbo.perms.command", Context::Global, true), "{set:?}");
    p.run(Some(bob), split_args("user Bob info"));
    let got = testing::messages(bob).join("\n");
    assert!(got.contains("Bob"), "{got}");
    p.run(Some(bob), split_args("group vip delete"));
    assert!(testing::messages(bob).join("\n").contains("permission"), "no right to delete groups");
    assert!(p.with(|s| s.engine.perms.group("vip").is_some()).unwrap());

    // Offline: the service answers from the data.
    let uuid = uuid_of(&players::get(bob).unwrap());
    let ask = |node: &str, context: &str| {
        let req = CheckOffline { uuid: uuid.replace('-', ""), node: node.into(), context: context.into() };
        let out = call(&p, METHOD_CHECK_OFFLINE, cbor(&req).unwrap()).unwrap();
        pumbo_sdk::ciborium::from_reader::<CheckOfflineAnswer, _>(out.as_slice()).unwrap().value
    };
    assert_eq!(ask("pumbo.proxy.glist", "server=lobby"), Some(true));
    assert_eq!(ask("pumbo.proxy.glist", "global"), None);
    assert_eq!(ask("pumboperms:user.info", "global"), Some(true), "both spellings are one node");

    // The export says that the proxy rules.
    let out = call(&p, METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
    let ans: PermissionsExport = pumbo_sdk::ciborium::from_reader(out.as_slice()).unwrap();
    let snap = export::parse(&ans.data).unwrap();
    assert!(snap.proxy_rules && snap.groups.iter().any(|g| g.name == "vip"));
    assert_eq!(snap.exported, 0, "stable text for the fingerprint");
    block_on(p.on_disconnect(bob));
    assert!(p.with(|s| s.loaded.is_empty()).unwrap());
}

fn contracts_request() -> pumbo_sdk::contracts::PermissionsExportRequest {
    pumbo_sdk::contracts::PermissionsExportRequest { server: "lobby".into(), groups: vec!["hubs".into()] }
}

/// A broken database: the proxy gets an error (its `on-load-failure`
/// decides), never an empty set that would take everything away.
#[test]
fn broken_database_fails_closed() {
    testing::reset();
    let p = PumboPerms::default();
    p.start(Err(pumbo_common::store::StoreError::new("disk on fire"))).unwrap();
    let id = testing::add_player("Steve", None);
    let r = block_on(p.on_permission_load(players::get(id).unwrap()));
    assert!(r.unwrap_err().contains("disk on fire"));
}

/// 1 000 changes with 20 players loaded: each one reaches the proxy, and a
/// set for one player takes well under a millisecond.
#[test]
fn many_changes() {
    let (p, _) = plugin(None);
    let ids: Vec<PlayerId> = (0..20).map(|i| testing::add_player(&format!("P{i}"), Some("lobby"))).collect();
    for id in &ids {
        load(&p, *id);
    }
    console(&p, "creategroup vip");
    let t = std::time::Instant::now();
    for i in 0..500 {
        console(&p, &format!("user P{} parent add vip server=lobby", i % 20));
        console(&p, &format!("user P{} parent remove vip server=lobby", i % 20));
    }
    let took = t.elapsed();
    let sent = testing::with(|h| h.replaced.len());
    assert!(sent >= 1000, "{sent}");
    eprintln!("[measure] 1000 changes, 20 players loaded: {took:?}, {sent} sets sent");
    let errors = logs().matches("did not take").count();
    assert_eq!(errors, 0);
}

#[test]
fn built_in_language_files_are_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lang");
    let written = Lang::write_templates(&setup::BUNDLES, &dir);
    assert!(written.is_empty(), "rewritten from the message bundles, commit them: {written:?}");
}
