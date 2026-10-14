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
    plugin_on(config, PermsStore::in_memory().unwrap())
}

fn plugin_on(config: Option<&str>, store: PermsStore) -> (PumboPerms, std::path::PathBuf) {
    testing::reset();
    testing::with(|h| h.servers = vec![server("lobby", &["hubs"]), server("survival", &[])]);
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
    let (p, _) = plugin(None);
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
    let ledger = || p.with(|s| s.engine.ledger(SOURCE).unwrap()).unwrap();
    assert_eq!(ledger().pending[0].name, "Jeb_");

    // Jeb_ joins for the first time with an online-mode account: the entries
    // waiting for the name.
    let jeb = testing::add_player("jeb_", Some("survival"));
    testing::with(|h| h.players.get_mut(&jeb).unwrap().online_mode = true);
    let set = load(&p, jeb);
    assert!(has(&set, "group.vip", Context::Global, true), "{set:?}");
    assert!(logs().contains("joined for the first time"), "{}", logs());
    assert!(ledger().pending.is_empty());

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

/// A server group named in mixed case in the proxy config still matches the
/// lower-case `group=` context.
#[test]
fn group_context_matches_server_group_in_any_case() {
    let (p, _) = plugin(None);
    testing::with(|h| h.servers = vec![server("lobby", &["Hubs"]), server("survival", &[])]);
    p.with(|s| s.engine.server_groups = server_groups()).unwrap();
    let bob = testing::add_player("Bob", Some("lobby"));
    load(&p, bob);
    console(&p, "user Bob permission set only.hubs true group=hubs");
    let set = last_replace(bob).expect("pushed after the change");
    assert!(has(&set, "only.hubs", Context::Server("lobby".into()), true), "{set:?}");
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
    block_on(p.run(Some(bob), split_args("user Bob info")));
    let got = testing::messages(bob).join("\n");
    assert!(got.contains("Bob"), "{got}");
    block_on(p.run(Some(bob), split_args("group vip delete")));
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

    // The export only says that the proxy rules.
    let out = call(&p, METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
    let ans: PermissionsExport = pumbo_sdk::ciborium::from_reader(out.as_slice()).unwrap();
    let snap = export::parse(&ans.data).unwrap();
    assert!(snap.proxy_rules && snap.groups.is_empty() && snap.users.is_empty(), "{snap:?}");
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
/// change (command, save, new set) takes well under a millisecond.
#[test]
#[ignore = "release benchmark"]
fn many_changes() {
    let (p, _) = plugin(None);
    let ids: Vec<PlayerId> = (0..20).map(|i| testing::add_player(&format!("P{i}"), Some("lobby"))).collect();
    for id in &ids {
        load(&p, *id);
    }
    console(&p, "creategroup vip");
    let start = std::time::Instant::now();
    for i in 0..500 {
        console(&p, &format!("user P{} parent add vip server=lobby", i % 20));
        console(&p, &format!("user P{} parent remove vip server=lobby", i % 20));
    }
    let took = start.elapsed();
    let sent = testing::with(|h| h.replaced.len());
    assert!(sent >= 1000, "{sent}");
    let errors = logs().matches("did not take").count();
    assert_eq!(errors, 0);
    println!("1000 changes with 20 players: {took:?}");
    assert!(took < std::time::Duration::from_millis(500), "{took:?}");
}

/// Change notices go only to players whose permission set has the notify node.
#[test]
fn notify_reaches_only_players_with_the_node() {
    let (p, _) = plugin(None);
    let staff = testing::add_player("Staff", Some("lobby"));
    let joe = testing::add_player("Joe", Some("lobby"));
    load(&p, staff);
    load(&p, joe);
    console(&p, "user Staff permission set pumbo.perms.log.notify");
    let (staff_before, joe_before) = (testing::messages(staff).len(), testing::messages(joe).len());
    console(&p, "creategroup vip");
    let got = testing::messages(staff);
    assert!(got.get(staff_before..).is_some_and(|new| new.iter().any(|m| m.contains("vip"))), "{got:?}");
    assert_eq!(testing::messages(joe).len(), joe_before);
}

const EVERYTHING: &str = r#"{"format":"pumboperms","version":1,
    "groups":[{"name":"default","data":{"permissions":[{"node":"*","value":true}]}}]}"#;

#[test]
fn file_method_rejects_other_plugins() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    let got = call_as(&p, "evil-plugin", METHOD_FILE, file(EVERYTHING));
    assert!(matches!(got, Err(CallReject::Rejected(_))), "{got:?}");
    assert!(!has(&load(&p, steve), "*", Context::Global, true));
    call_as(&p, "proxy", METHOD_FILE, file(EVERYTHING)).unwrap();
    assert!(has(&load(&p, steve), "*", Context::Global, true), "the host may hand over permissions.yml");
}

#[test]
fn export_method_rejects_other_plugins() {
    let (p, _) = plugin(None);
    let got = call_as(&p, "evil-plugin", METHOD_EXPORT, cbor(&contracts_request()).unwrap());
    assert!(matches!(got, Err(CallReject::Rejected(_))), "{got:?}");
    call_as(&p, "proxy", METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
}

#[test]
fn pending_import_skips_offline_accounts() {
    let (p, _) = plugin(None);
    let data = r#"{"format":"pumboperms","version":1,
        "groups":[{"name":"mod","data":{"permissions":[{"node":"pumbo.bans.ban","value":true}]}}],
        "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#;
    call_as(&p, "proxy", METHOD_FILE, file(data)).unwrap();
    let offline = testing::add_player("Mod1", Some("lobby"));
    assert!(!has(&load(&p, offline), "group.mod", Context::Global, true), "an offline account took the rank");
    let online = testing::add_player("Mod1", Some("lobby"));
    testing::with(|h| h.players.get_mut(&online).unwrap().online_mode = true);
    assert!(has(&load(&p, online), "group.mod", Context::Global, true));
}

/// A `config.yml` that is not valid YAML at start: the last good copy, not the
/// defaults (which would import permissions.yml again, for one).
#[test]
fn broken_config_at_start_keeps_last_good() {
    let (_, dir) = plugin(Some("language: pl\nimport:\n  enabled: false\n"));
    std::fs::write(dir.join("config.yml"), "language: [pl\n").unwrap();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Ok(PermsStore::in_memory().unwrap())).unwrap();
    assert_eq!(p.with(|s| (s.cfg.language.clone(), s.cfg.import.enabled)).unwrap(), ("pl".into(), false));
    assert!(testing::with(|h| h.logs.iter().any(|(_, l)| l.contains(common_config::LAST_GOOD))), "{}", logs());
    // Without the copy the plugin does not start.
    std::fs::remove_file(dir.join(common_config::LAST_GOOD)).unwrap();
    let p = PumboPerms {
        state: RefCell::new(None),
        config_dir: dir.to_string_lossy().into(),
        data_dir: dir.to_string_lossy().into(),
    };
    assert!(p.start(Ok(PermsStore::in_memory().unwrap())).unwrap_err().contains("not valid YAML"));
    let _ = std::fs::remove_file(dir.join("config.yml"));
}

#[test]
fn built_in_language_files_are_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/lang");
    let written = Lang::write_templates(&setup::BUNDLES, &dir);
    assert!(written.is_empty(), "rewritten from the message bundles, commit them: {written:?}");
}

/// Every answer for the same data is equal: the core, the proxy's table,
/// the PumboBridge perm-set of a backend and Pumpkin reading its attributes.
/// Where the core has no answer, the hosts have none either; a catalog child
/// under a parent node set to true is false on the hosts (Pumpkin reads an
/// exact parent like a wildcard), and on the proxy too when the data has it.
#[test]
fn host_and_core_agree() {
    use std::collections::HashMap;

    use pumbo_contracts::permissions::{Layers, PermContext, PermissionEntry as Entry, backend_set, levels, resolve};
    use pumbo_perms_core::config::CoreCfg;
    use pumbo_perms_core::context::Contexts;
    use pumbo_perms_core::engine::Engine;
    use pumbo_perms_core::host::{Catalog, wanted};
    use pumbo_perms_core::model::{ParentNode, PermNode};
    use pumbo_perms_core::node;
    use pumbo_perms_core::testing::pumpkin_has;

    let mut e = Engine::in_memory(CoreCfg::default(), Lang::load(&setup::BUNDLES, "en", None).0);
    // As the plugin keeps them: lower case, while players report `Survival`.
    e.server_groups = [("lobby".to_string(), vec!["hub".to_string()]), ("survival".to_string(), Vec::new())].into();
    let groups: &[(&str, &[(&str, bool)])] = &[
        ("default", &[]),
        ("mod", &[("pumbo.bans.*", true), ("pumbo.bans.exempt.*", false), ("pumbo.bans.notify", true)]),
        ("admin", &[("pumbo.*", true)]),
        ("helper", &[("pumbo.bans.limit.helper", true)]),
        ("vip", &[("minecraft:command.random", true), ("myplugin:home", true)]),
        ("owner", &[("*", true), ("minecraft:command.*", false)]),
        (
            "jr",
            &[
                ("pumbo.bans.*", true),
                ("pumbo.bans.ban", false),
                ("minecraft:command.*", true),
                ("minecraft:command.kick", false),
            ],
        ),
    ];
    for (name, nodes) in groups {
        if e.perms.group(name).is_none() {
            e.perms.create_group(name).unwrap();
        }
        e.perms
            .edit_group(name, |g| {
                for (n, v) in *nodes {
                    g.data.set_permission(PermNode {
                        node: (*n).into(),
                        value: *v,
                        context: Contexts::global(),
                        expiry: None,
                    });
                }
            })
            .unwrap();
    }
    // (group, node, value, context)
    let contextual: &[(&str, &str, bool, &str)] = &[
        ("admin", "myplugin:home.others", true, ""),
        ("helper", "minecraft:command.seed", true, "server=lobby"),
        ("helper", "minecraft:command.list", true, "group=hub"),
    ];
    for (group, n, v, context) in contextual {
        let context =
            if context.is_empty() { Contexts::global() } else { Contexts::parse(&[context.to_string()]).unwrap() };
        let perm = PermNode { node: (*n).into(), value: *v, context, expiry: None };
        e.perms.edit_group(group, |g| g.data.set_permission(perm)).unwrap();
    }
    // (uuid, parent, context of the parent, operator level)
    let players: &[(&str, &str, &str, u8)] = &[
        ("00000000-0000-0000-0000-000000000001", "mod", "", 0),
        ("00000000-0000-0000-0000-000000000002", "admin", "", 0),
        ("00000000-0000-0000-0000-000000000003", "helper", "", 0),
        ("00000000-0000-0000-0000-000000000004", "vip", "", 0),
        ("00000000-0000-0000-0000-000000000005", "vip", "server=survival", 0),
        ("00000000-0000-0000-0000-000000000006", "default", "", 3),
        ("00000000-0000-0000-0000-000000000007", "owner", "", 0),
        ("00000000-0000-0000-0000-000000000008", "jr", "", 0),
        ("00000000-0000-0000-0000-000000000009", "mod", "", 3),
    ];
    for (uuid, group, context, _) in players {
        let context =
            if context.is_empty() { Contexts::global() } else { Contexts::parse(&[context.to_string()]).unwrap() };
        let parent = ParentNode { group: (*group).into(), context, expiry: None };
        e.perms.edit_user(uuid, "Player", |d| d.add_parent(parent));
    }

    let bridge_catalog: Vec<String> = pumbo_common::pumpkin::COMMAND_NODES
        .iter()
        .chain(pumbo_common::pumpkin::PUMBO_NODES)
        .map(|n| n.to_string())
        .collect();
    let mut catalog = Catalog::pumpkin();
    catalog.refresh(&e.perms, pumbo_perms_core::command::own_nodes().iter().map(|n| node::host_form(n)));
    let mut nodes: Vec<String> = catalog.nodes().iter().filter(|n| !node::is_wildcard(n)).cloned().collect();
    nodes.extend(
        ["pumbobans:ban", "pumbobans:exempt.ban", "pumbobans:limit.helper", "pumbobans:notify"].map(String::from),
    );
    nodes.sort();
    nodes.dedup();
    // `pumboperms:command` is whether /pp shows: operator levels count only on Pumpkin.
    let command = node::host_form(&pumbo_common::command::permission(command::PLUGIN, "command"));
    // The proxy gets the same false as the hosts for these (data nodes, decision 10).
    let data_nodes: Vec<String> = e.perms.all_nodes().iter().map(|n| node::host_form(n)).collect();

    let mut wrong = Vec::new();
    for (uuid, _, _, op_level) in players {
        // The proxy's engine has no server of its own; Pumpkin's is set per place below.
        e.cfg.context.server.clear();
        let provider: Vec<Entry> = setup::provider_set(&mut e, uuid, 0)
            .into_iter()
            .map(|(server, node, value)| Entry {
                node,
                value,
                context: server.map_or(PermContext::Global, |s| PermContext::server(&s)),
            })
            .collect();
        let layers = Layers { provider: &provider, ..Layers::default() };
        for place in ["", "lobby", "Survival"] {
            let server_groups = e.server_groups.get(&place.to_lowercase()).cloned().unwrap_or_default();
            let lv = levels((!place.is_empty()).then_some(place), &server_groups);
            let bridge: HashMap<String, bool> = backend_set(layers, &bridge_catalog, &lv).into_iter().collect();
            e.cfg.context.server = place.to_lowercase();
            let attributes = wanted(&mut e, uuid, "world", *op_level, 0, &catalog);
            let q = e.place(place, "");
            let shows = |e: &mut Engine, op_level: u8| {
                let who = Who { server: place.to_string(), ..Who::player("", uuid, op_level, "world") };
                Some(command::allowed_any(e, &who, 0))
            };
            let (shows_proxy, shows_pumpkin) = (shows(&mut e, 0), shows(&mut e, *op_level));
            let mut core = |n: &str| e.perms.check(uuid, &node::normalize(n).unwrap_or_default(), &q, 0);
            for host in &nodes {
                let want = core(host);
                let parent_true = host.split_once(':').is_some_and(|(ns, key)| {
                    key.match_indices('.').any(|(i, _)| core(&format!("{ns}:{}", &key[..i])) == Some(true))
                });
                let on_host = if want.is_none() && parent_true { Some(false) } else { want };
                let canonical = node::normalize(host).unwrap_or_default();
                let on_proxy = if data_nodes.contains(host) { on_host } else { want };
                let mut answers = vec![
                    ("proxy", resolve(layers, &canonical, &lv).map(|d| d.value), on_proxy),
                    ("pumpkin", pumpkin_has(&attributes, host), on_host),
                    ("bridge", pumpkin_has(&bridge, host), on_host),
                ];
                if *host == command {
                    answers = vec![
                        ("proxy", resolve(layers, &canonical, &lv).map(|d| d.value), shows_proxy),
                        ("bridge", pumpkin_has(&bridge, host), shows_proxy),
                        ("pumpkin", pumpkin_has(&attributes, host), shows_pumpkin),
                    ];
                }
                for (side, got, expected) in answers {
                    if got != expected {
                        wrong.push(format!("{uuid} {place:?} {host}: core {want:?}, {side} {got:?}"));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{} disagreements:\n{}", wrong.len(), wrong.join("\n"));
    let lobby = e.place("lobby", "");
    let helper = players[2].0;
    assert_eq!(e.perms.check(helper, "minecraft:command.list", &lobby, 0), Some(true), "server group reaches lobby");
}

#[test]
fn proxy_refuses_world_context() {
    let (p, _) = plugin(None);
    console(&p, "creategroup vip");
    let refused = |p: &PumboPerms, line: &str| {
        console(p, line);
        let out = testing::with(|h| h.logs.last().map(|(_, l)| l.clone())).unwrap_or_default();
        assert!(out.contains("does not see worlds"), "{line}: {out}");
    };
    refused(&p, "group vip permission set fly true world=nether");
    refused(&p, "group vip parent add default world=nether");
    refused(&p, "group vip meta set color red world=nether");
    refused(&p, "group vip permission check fly world=nether");
    assert!(p.with(|s| s.engine.perms.group("vip").unwrap().data.is_empty()).unwrap());
}

#[test]
fn export_answer_fits_the_host_payload_limit() {
    let (p, _) = plugin(None);
    console(&p, "creategroup vip");
    p.with(|s| {
        let mut dirty = pumbo_perms_core::perms::Dirty::default();
        for i in 0..3000u64 {
            let uuid = Uuid::from_high_low(1, i).to_string();
            let parent = pumbo_perms_core::model::ParentNode {
                group: "vip".into(),
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            let (_, d) = s.engine.perms.edit_user(&uuid, &format!("Player{i}"), |d| d.add_parent(parent));
            dirty.merge(d);
        }
        s.engine.commit(&dirty, None).unwrap();
    })
    .unwrap();
    let out = call_as(&p, "proxy", METHOD_EXPORT, cbor(&contracts_request()).unwrap()).unwrap();
    // Only the flag, whatever the database holds (the host takes up to 256 KiB).
    assert!(out.len() < 1024, "{} bytes", out.len());
}

#[test]
fn expiry_reaches_the_proxy_when_the_store_fails() {
    let backend = pumbo_common::store::FaultyBackend::default();
    let writes_fail = backend.fail_writes.clone();
    let (p, _) = plugin_on(None, PermsStore::new(pumbo_common::store::Store::new(backend)).unwrap());
    let bob = testing::add_player("Bob", Some("lobby"));
    load(&p, bob);
    console(&p, "creategroup vip");
    console(&p, "user Bob parent addtemp vip 1s");
    assert!(has(&last_replace(bob).unwrap(), "group.vip", Context::Global, true));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    writes_fail.store(u32::MAX, std::sync::atomic::Ordering::SeqCst);
    let timer = p.with(|s| s.timer).unwrap();
    block_on(p.on_timer(timer));
    let set = last_replace(bob).unwrap();
    assert!(!has(&set, "group.vip", Context::Global, true), "expired vip still on the proxy: {set:?}");
}

#[test]
fn info_method_answers_for_a_server() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    load(&p, steve);
    console(&p, "creategroup vip");
    console(&p, "group vip meta addprefix 10 \"[VIP] \" server=lobby");
    console(&p, "user Steve parent add vip");
    let uuid = uuid_of(&players::get(steve).unwrap()).replace('-', "");
    let ask = |server: &str| {
        let req = PermissionsInfoRequest { uuid: uuid.clone(), server: server.into(), groups: vec!["hubs".into()] };
        let out = call(&p, METHOD_INFO, cbor(&req).unwrap()).unwrap();
        let ans: PermissionsInfo = pumbo_sdk::ciborium::from_reader(out.as_slice()).unwrap();
        serde_json::from_str::<ipc::Info>(&ans.data.unwrap()).unwrap()
    };
    let lobby = ask("Lobby");
    assert_eq!((lobby.rank.as_str(), lobby.prefix.as_str()), ("vip", "[VIP] "));
    assert_eq!(ask("survival").prefix, "");
    let req = PermissionsInfoRequest { uuid, server: "lobby".into(), groups: Vec::new() };
    let got = call_as(&p, "other-plugin", METHOD_INFO, cbor(&req).unwrap());
    assert!(matches!(got, Err(CallReject::Rejected(_))), "{got:?}");
}

/// A change that leaves the decisions alone but changes what the player
/// shows (a prefix) still reaches the proxy, which then asks for the info.
#[test]
fn prefix_change_alone_reaches_the_proxy() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    load(&p, steve);
    let before = testing::with(|h| h.replaced.len());
    console(&p, "user Steve meta addprefix 10 \"[S] \"");
    assert_eq!(testing::with(|h| h.replaced.len()), before + 1);
    console(&p, "user Steve meta addprefix 10 \"[S] \"");
    assert_eq!(testing::with(|h| h.replaced.len()), before + 1, "the same prefix again changes nothing");
}

/// The info kept for a player is the one the host asks for: on the
/// player's server with that server's groups.
#[test]
fn group_context_prefix_reaches_the_proxy() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    load(&p, steve);
    let before = testing::with(|h| h.replaced.len());
    console(&p, "user Steve meta addprefix 10 \"[H] \" group=hubs");
    assert_eq!(testing::with(|h| h.replaced.len()), before + 1);
    let uuid = uuid_of(&players::get(steve).unwrap());
    let kept = p.with(|s| s.loaded.get(&steve).unwrap().2.clone()).unwrap();
    let req = PermissionsInfoRequest { uuid, server: "lobby".into(), groups: vec!["hubs".into()] };
    let out = call(&p, METHOD_INFO, cbor(&req).unwrap()).unwrap();
    let ans: PermissionsInfo = pumbo_sdk::ciborium::from_reader(out.as_slice()).unwrap();
    assert_eq!(kept, ans.data.unwrap());
    // A player between servers: the host asks with no server and no groups.
    let walker = testing::add_player("Walker", None);
    load(&p, walker);
    let kept = p.with(|s| s.loaded.get(&walker).unwrap().2.clone()).unwrap();
    let req = PermissionsInfoRequest {
        uuid: uuid_of(&players::get(walker).unwrap()),
        server: String::new(),
        groups: Vec::new(),
    };
    let out = call(&p, METHOD_INFO, cbor(&req).unwrap()).unwrap();
    let ans: PermissionsInfo = pumbo_sdk::ciborium::from_reader(out.as_slice()).unwrap();
    assert_eq!(kept, ans.data.unwrap());
}

#[test]
fn export_on_broken_database_keeps_existing_file() {
    testing::reset();
    let dir = std::env::temp_dir().join(format!("pumbo-perms-prox-broken-export-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("exports")).unwrap();
    let path = dir.join("exports/nightly.json");
    std::fs::write(&path, "precious").unwrap();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(Err(pumbo_common::store::StoreError::new("disk on fire"))).unwrap();
    console(&p, "export nightly");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "precious");
    console(&p, "export fresh");
    assert!(!dir.join("exports/fresh.json").exists(), "no empty export of a broken database");
}

/// An offline account that joined before the import does not get the rank of
/// its nickname: the name table counts only accounts Mojang checked.
#[test]
fn offline_join_then_import_run_gets_nothing() {
    let (p, _) = plugin(Some("import:\n  enabled: false\n"));
    let offline = testing::add_player("Mod1", Some("lobby"));
    load(&p, offline);
    let data = r#"{"format":"pumboperms","version":1,
        "groups":[{"name":"mod","data":{"permissions":[{"node":"pumbo.bans.ban","value":true}]}}],
        "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#;
    call(&p, METHOD_FILE, file(data)).unwrap();
    console(&p, "import run file");
    assert!(logs().contains("players are known only by name") && logs().contains("Mod1"), "{}", logs());
    assert!(!has(&load(&p, offline), "group.mod", Context::Global, true), "an offline account took the rank");
    assert_eq!(p.with(|s| s.engine.ledger(SOURCE).unwrap().pending.len()).unwrap(), 1);
}

/// The ledgers of a database from before schema 2 move into it at start.
#[test]
fn ledger_file_moves_into_the_database() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-prox-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("imports")).unwrap();
    std::fs::write(dir.join("imports/file.json"), r#"{"fingerprint":"f1","at":7,"entries":[]}"#).unwrap();
    let store = pumbo_common::store::Store::open(dir.join("perms.redb")).unwrap();
    store.write(|tx| pumbo_common::store::WriteExt::put_u64(tx, pumbo_perms_core::store::META, "schema", 1)).unwrap();
    testing::reset();
    let d = dir.to_string_lossy().to_string();
    let p = PumboPerms { state: RefCell::new(None), config_dir: d.clone(), data_dir: d };
    p.start(PermsStore::new(store)).unwrap();
    assert!(logs().contains("schema 1 -> 2"), "{}", logs());
    assert_eq!(p.with(|s| s.engine.ledger(SOURCE).unwrap().fingerprint).unwrap(), "f1");
    assert!(dir.join("imports/file.json.migrated").exists());
}

/// A network at scale: 10 servers, 20 groups of 50 nodes (some per server),
/// 3000 players in the database and `online` of them loaded.
fn at_scale(online: usize) -> (PumboPerms, Vec<PlayerId>) {
    use pumbo_perms_core::context::Contexts;
    use pumbo_perms_core::model::{ParentNode, PermNode};
    use pumbo_perms_core::perms::Dirty;
    // On a database file, as the proxy runs it.
    let dir = std::env::temp_dir().join(format!("pumbo-perms-scale-{}-{online}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (p, _) = plugin_on(None, PermsStore::new(Store::open(dir.join("perms.redb")).unwrap()).unwrap());
    testing::with(|h| h.servers = (0..10).map(|i| server(&format!("s{i}"), &[])).collect());
    let on = |s: usize| Contexts::parse(&[format!("server=s{s}")]).unwrap();
    let ids: Vec<PlayerId> =
        (0..online).map(|k| testing::add_player(&format!("P{k}"), Some(&format!("s{}", k % 10)))).collect();
    let uuids: Vec<String> = ids.iter().map(|id| uuid_of(&players::get(*id).unwrap())).collect();
    p.with(|s| {
        s.engine.server_groups = server_groups();
        let perms = &mut s.engine.perms;
        let mut dirty = Dirty::default();
        for g in 0..20 {
            let name = format!("g{g}");
            dirty.merge(perms.create_group(&name).unwrap());
            let (_, d) = perms
                .edit_group(&name, |gr| {
                    gr.weight = g;
                    gr.data.add_parent(ParentNode {
                        group: "default".into(),
                        context: Contexts::global(),
                        expiry: None,
                    });
                    for n in 0..50 {
                        let node = match n % 5 {
                            0 => format!("minecraft:command.c{g}n{n}"),
                            1 => format!("pumbo.p{g}.a{n}"),
                            _ => format!("plugin{g}.node{n}"),
                        };
                        let context = if n % 10 == 9 { on(n % 10) } else { Contexts::global() };
                        gr.data.set_permission(PermNode { node, value: n % 7 != 0, context, expiry: None });
                    }
                })
                .unwrap();
            dirty.merge(d);
        }
        let parent = |g: usize, context: Contexts| ParentNode { group: format!("g{}", g % 20), context, expiry: None };
        for k in 0..3000 {
            let uuid = format!("00000000-0000-4000-8000-{k:012}");
            dirty.merge(perms.edit_user(&uuid, "", |d| d.add_parent(parent(k, Contexts::global()))).1);
        }
        for (k, uuid) in uuids.iter().enumerate() {
            let (_, d) = perms.edit_user(uuid, &format!("P{k}"), |d| {
                d.add_parent(parent(k, Contexts::global()));
                d.add_parent(parent(k + 7, on(k % 10)));
            });
            dirty.merge(d);
        }
        s.engine.commit(&dirty, None).unwrap();
    });
    for id in &ids {
        load(&p, *id);
    }
    (p, ids)
}

/// A change in `default` with 300 players on 10 servers: well inside the
/// proxy's 200 ms plugin budget, natively (the median of 20 rounds; the
/// slowest is printed too).
#[test]
#[ignore = "release benchmark"]
fn push_all_scale() {
    let (p, ids) = at_scale(300);
    let mut took = Vec::new();
    for round in 0..20 {
        testing::with(|h| h.replaced.clear());
        p.with(|s| {
            let node = pumbo_perms_core::model::PermNode {
                node: "scale.extra".into(),
                value: round % 2 == 0,
                context: pumbo_perms_core::context::Contexts::global(),
                expiry: None,
            };
            let (_, d) = s.engine.perms.edit_group("default", |g| g.data.set_permission(node)).unwrap();
            s.engine.commit(&d, None).unwrap();
        });
        let start = std::time::Instant::now();
        block_on(p.push_all());
        took.push(start.elapsed());
        assert_eq!(testing::with(|h| h.replaced.len()), ids.len(), "every player is in default");
    }
    took.sort();
    let (median, worst) = (took[took.len() / 2], took[took.len() - 1]);
    println!("push_all, 300 players on 10 servers, a change in default: median {median:?}, slowest {worst:?}");
    assert!(median < std::time::Duration::from_millis(50), "{median:?}");
}

/// A change of one player recomputes that player alone.
#[test]
fn push_all_recomputes_only_affected() {
    let (p, ids) = at_scale(30);
    testing::with(|h| h.replaced.clear());
    console(&p, "user P7 permission set only.mine");
    let sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    assert_eq!(sent, [ids[7]]);
    // A group reaches its members: g3 is the rank of P3 and P23, and a parent of P16 on s6.
    testing::with(|h| h.replaced.clear());
    console(&p, "group g3 permission set only.g3");
    let mut sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    sent.sort();
    assert_eq!(sent, [ids[3], ids[16], ids[23]]);
    // New servers: everyone is computed again, nobody gets the same set twice.
    testing::with(|h| h.replaced.clear());
    testing::with(|h| h.servers.push(server("s10", &[])));
    block_on(p.push_all());
    assert!(testing::with(|h| h.replaced.is_empty()), "nothing names s10");
    // A parent of default reaches a player who is in default only.
    let solo = testing::add_player("Solo", Some("s0"));
    load(&p, solo);
    console(&p, "creategroup base");
    console(&p, "group default parent add base");
    testing::with(|h| h.replaced.clear());
    console(&p, "group base permission set only.base");
    let sent: Vec<PlayerId> = testing::with(|h| h.replaced.iter().map(|(id, _)| *id).collect());
    assert!(sent.contains(&solo) && sent.len() == ids.len() + 1, "{sent:?}");
}

/// The proxy's permissions.yml as the host sends it: 3000 players in vip and
/// `extra` more nodes in vip.
fn big_file(extra: usize) -> Vec<u8> {
    let nodes: Vec<String> =
        (0..=extra).map(|i| format!("{{\"node\":\"minecraft:command.n{i}\",\"value\":true}}")).collect();
    let users: Vec<String> = (0..3000)
        .map(|i| {
            format!("{{\"uuid\":\"00000000-0000-4000-8000-{i:012}\",\"data\":{{\"parents\":[{{\"group\":\"vip\"}}]}}}}")
        })
        .collect();
    file(&format!(
        r#"{{"format":"pumboperms","version":1,"groups":[{{"name":"vip","data":{{"permissions":[{}]}}}}],"users":[{}]}}"#,
        nodes.join(","),
        users.join(",")
    ))
}

/// permissions.yml with 3000 players on a database file: the first import at
/// start and `/pp import run file` after it changed stay well inside the
/// proxy's 200 ms plugin budget, natively; reading the file, the import and
/// each portion of new sets are separate pieces of work there.
#[test]
#[ignore = "release benchmark"]
fn import_of_3000_players_scale() {
    let dir = std::env::temp_dir().join(format!("pumbo-perms-import-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = PermsStore::new(Store::open(dir.join("perms.redb")).unwrap()).unwrap();
    let (p, _) = plugin_on(Some("import:\n  follow: false\n"), store);
    let ids: Vec<PlayerId> = (0..100).map(|k| testing::add_player(&format!("P{k}"), Some("lobby"))).collect();
    for id in &ids {
        load(&p, *id);
    }
    let start = std::time::Instant::now();
    call(&p, METHOD_FILE, big_file(0)).unwrap();
    let first = start.elapsed();
    assert!(logs().contains("Imported file"), "{}", logs());
    call(&p, METHOD_FILE, big_file(1)).unwrap();
    let start = std::time::Instant::now();
    console(&p, "import run file");
    let again = start.elapsed();
    assert_eq!(p.with(|s| s.engine.perms.users().count()).unwrap(), 3000);
    println!("import of 3000 players: first {first:?}, again after a change {again:?}");
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
    // The first import includes reading the file and new sets for the 100 players.
    assert!(first < std::time::Duration::from_millis(100) && again < std::time::Duration::from_millis(50));
}

/// Resident memory of this process in KB (`ps`).
fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or(0)
}

/// The memory the plugin needs at scale (300 players on 10 servers, 3000 in
/// the database, an import of 3000 more), as the growth of this process: a
/// native build has 8-byte pointers, so the plugin in WASM needs less.
/// Run alone; other tests in the same process would count too.
#[test]
#[ignore = "release benchmark"]
fn memory_at_scale() {
    let before = rss_kb();
    let (p, _) = at_scale(300);
    call(&p, METHOD_FILE, big_file(0)).unwrap();
    for _ in 0..3 {
        console(&p, "group default permission set scale.extra");
        console(&p, "group default permission unset scale.extra");
        // The fake host keeps every set it got; the proxy does not.
        testing::with(|h| h.replaced.clear());
    }
    let grown = rss_kb().saturating_sub(before);
    println!("memory at scale: the process grew by {} MB", grown / 1024);
    assert!(grown < 64 * 1024, "{grown} KB");
}
