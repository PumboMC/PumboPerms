//! The permission sets the proxy gets, contexts, placeholders and the info call.

use super::*;

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

/// A prefix's codes end with the value: `&r` inside it and its end give the
/// chat template its own style back.
#[test]
fn placeholder_values_close_their_tags() {
    let (p, _) = plugin(None);
    let steve = testing::add_player("Steve", Some("lobby"));
    load(&p, steve);
    console(&p, "creategroup admin");
    console(&p, "group admin setdisplayname &4&lAdmin");
    console(&p, "user Steve parent add admin");
    console(&p, "user Steve meta setprefix 10 \"&c[Admin]&r <x> \"");
    console(&p, "user Steve meta setsuffix 10 &#00ff00!");
    let ask = |key: &str| PlaceholderRequest {
        key: key.into(),
        arg: None,
        player: Some(steve),
        context: pumbo_sdk::PlayerContext { server: Some("lobby".into()), groups: Vec::new() },
    };
    let got = block_on(p.on_placeholder(vec![ask("prefix"), ask("suffix"), ask("rank"), ask("group")]));
    let shown: Vec<String> = got
        .iter()
        .map(|t| match t {
            Some(Text::Mini(s)) => format!("mini {s}"),
            Some(Text::Legacy(s)) => format!("legacy {s}"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shown,
        [
            "mini <color:#ff5555>[Admin]</color> \\<x> ",
            "mini <color:#00ff00>!</color>",
            "mini <color:#aa0000><bold>Admin</bold></color>",
            "legacy admin",
        ]
    );
}
