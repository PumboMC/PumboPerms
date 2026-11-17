//! Answers to PumpkinPIE's placeholder requests, also while the proxy rules.

use pumbo_perms_core::ipc;

use super::*;
use crate::pie::{self, NAMES, Request};

fn ask(name: &str, viewer: Option<&str>, argument: Option<&str>) -> Request {
    Request {
        namespace: pie::NAMESPACE.into(),
        name: name.into(),
        viewer: viewer.map(str::to_string),
        argument: argument.map(str::to_string),
    }
}

/// The value inside a reply; `None` for a decline.
fn value(reply: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(reply).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["on_request"].as_object().expect("a body").get("value").map(|x| x.as_str().unwrap().to_string())
}

/// Steve online in `world_nether`, in `vip` (weight 10) with a prefix, a
/// suffix, a display name and the meta key `color`.
fn steve_with_a_rank(rt: &mut Rt) {
    rt.join(U, Online { name: "Steve".into(), world: "world_nether".into(), op_level: 0 }, 0);
    let (_, d) = rt
        .engine
        .perms
        .edit_group("vip", |g| {
            g.display_name = "VIP".into();
            g.weight = 10;
            for (kind, key, value) in [
                (pumbo_perms_core::model::MetaKind::Prefix, "", "[VIP] "),
                (pumbo_perms_core::model::MetaKind::Suffix, "", " *"),
                (pumbo_perms_core::model::MetaKind::Meta, "color", "gold"),
            ] {
                g.data.meta.push(pumbo_perms_core::model::MetaNode {
                    kind,
                    key: key.into(),
                    priority: 10,
                    value: value.into(),
                    context: pumbo_perms_core::context::Contexts::global(),
                    expiry: None,
                });
            }
        })
        .unwrap();
    rt.engine.commit(&d, None).unwrap();
    give(rt, "vip", "some.node", None);
}

#[test]
fn on_request_answers_like_fill() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let q = rt.engine.query("world_nether");
    let info = ipc::info(&mut rt.engine, U, &q, 0).unwrap();
    assert_eq!(info.rank_display, "VIP");
    for name in NAMES {
        let argument = (name == "meta").then_some("color");
        let token = match argument {
            Some(a) => format!("%pumboperms_{name}:{a}%"),
            None => format!("%pumboperms_{name}%"),
        };
        let got = rt.pie_value(&ask(name, Some("Steve"), argument), 0);
        assert_eq!(got.as_deref(), Some(ipc::fill(&info, &token).as_str()), "{name}");
    }
    assert_eq!(rt.pie_value(&ask("meta", Some("Steve"), Some("nope")), 0), None);
    assert_eq!(rt.pie_value(&ask("meta", Some("Steve"), None), 0), None);
}

#[test]
fn mixed_case_namespace_answers() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let req = Request { namespace: "PumboPerms".into(), ..ask("rank", Some("steve"), None) };
    assert_eq!(rt.pie_value(&req, 0).as_deref(), Some("VIP"));
}

#[test]
fn on_request_reads_the_world_of_the_player() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let nether = pumbo_perms_core::context::Contexts::parse(&["world=world_nether".to_string()]).unwrap();
    let (_, d) = rt
        .engine
        .perms
        .edit_group("vip", |g| {
            g.data.meta.push(pumbo_perms_core::model::MetaNode {
                kind: pumbo_perms_core::model::MetaKind::Prefix,
                key: String::new(),
                priority: 20,
                value: "[NETHER] ".into(),
                context: nether,
                expiry: None,
            });
        })
        .unwrap();
    rt.engine.commit(&d, None).unwrap();
    assert_eq!(rt.pie_value(&ask("prefix", Some("Steve"), None), 0).as_deref(), Some("[NETHER] "));
}

#[test]
fn offline_or_missing_viewer_declines() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    assert_eq!(rt.pie_value(&ask("rank", None, None), 0), None);
    assert_eq!(rt.pie_value(&ask("rank", Some("Alex"), None), 0), None);
    rt.leave(U, 0);
    assert_eq!(rt.pie_value(&ask("rank", Some("Steve"), None), 0), None);
}

#[test]
fn other_namespace_or_unknown_name_declines() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let other = Request { namespace: "perm".into(), ..ask("prefix", Some("Steve"), None) };
    assert_eq!(rt.pie_value(&other, 0), None);
    assert_eq!(rt.pie_value(&ask("nope", Some("Steve"), None), 0), None);
}

#[test]
fn bad_json_gets_an_error_reply() {
    // Not PumpkinPIE's: the usual answers, which say what is wrong.
    assert_eq!(pie::request(b"{oops"), None);
    let mut rt = rt();
    let reply: serde_json::Value =
        serde_json::from_slice(&ipc_reply(b"{oops", Some(&mut rt.engine), 0).unwrap()).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().is_some_and(|e| e.starts_with("bad request")), "{reply}");
}

#[test]
fn busy_state_declines_without_error() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    install(rt);
    let req = ask("rank", Some("Steve"), None);
    assert_eq!(value(&Rt::pie_reply(&req, 0)).as_deref(), Some("VIP"));
    // A request that arrives while the state is in use (a nested call).
    let nested = with(|_| Rt::pie_reply(&req, 0)).unwrap();
    assert_eq!(value(&nested), None);
    uninstall();
    assert_eq!(value(&Rt::pie_reply(&req, 0)), None, "no state at all");
}

#[test]
fn passive_mode_answers_from_bridge_info() {
    let mut rt = rt();
    rt.join(U, steve(), 0);
    install(rt);
    let from_proxy = ipc::Info {
        name: "Steve".into(),
        rank: "admin".into(),
        rank_display: "Admin".into(),
        prefix: "[A] ".into(),
        suffix: String::new(),
        groups: vec!["admin".into()],
        meta: [("color".to_string(), "red".to_string())].into(),
        weight: 100,
    };
    let data = serde_json::to_string(&from_proxy).unwrap();
    let asked = std::cell::RefCell::new(Vec::new());
    let bridge = |uuid: &str| {
        asked.borrow_mut().push(uuid.to_string());
        Some(data.clone())
    };
    assert_eq!(value(&Rt::pie_passive_reply(&ask("prefix", Some("Steve"), None), bridge)).as_deref(), Some("[A] "));
    assert_eq!(
        value(&Rt::pie_passive_reply(&ask("meta", Some("Steve"), Some("color")), bridge)).as_deref(),
        Some("red")
    );
    assert_eq!(asked.borrow().as_slice(), [U, U]);
    // Nobody online by that name, or a name PumboPerms does not have: no question to the bridge.
    assert_eq!(value(&Rt::pie_passive_reply(&ask("prefix", Some("Alex"), None), bridge)), None);
    assert_eq!(value(&Rt::pie_passive_reply(&ask("nope", Some("Steve"), None), bridge)), None);
    assert_eq!(asked.borrow().len(), 2);
    // Nothing from the bridge yet.
    assert_eq!(value(&Rt::pie_passive_reply(&ask("prefix", Some("Steve"), None), |_| None)), None);
    uninstall();
}

#[test]
fn info_carries_the_weight_of_the_primary_group() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let q = rt.engine.query("world_nether");
    let info = ipc::info(&mut rt.engine, U, &q, 0).unwrap();
    assert_eq!((info.rank.as_str(), info.weight), ("vip", 10));
    let reply: serde_json::Value = serde_json::from_slice(
        &ipc_reply(format!(r#"{{"op":"info","uuid":"{U}"}}"#).as_bytes(), Some(&mut rt.engine), 0).unwrap(),
    )
    .unwrap();
    assert_eq!(reply["weight"], 10, "{reply}");
    // Below zero too; without a group of their own, the weight of `default`.
    let (_, d) = rt.engine.perms.edit_group("vip", |g| g.weight = -5).unwrap();
    rt.engine.commit(&d, None).unwrap();
    assert_eq!(ipc::info(&mut rt.engine, U, &q, 0).unwrap().weight, -5);
    const ALEX: &str = "00000000-0000-0000-0000-000000000002";
    rt.join(ALEX, Online { name: "Alex".into(), world: "world".into(), op_level: 0 }, 0);
    let alex = ipc::info(&mut rt.engine, ALEX, &q, 0).unwrap();
    assert_eq!((alex.rank.as_str(), alex.weight), ("default", 0));
}

#[test]
fn weight_placeholder_matches_info() {
    let mut rt = rt();
    steve_with_a_rank(&mut rt);
    let q = rt.engine.query("world_nether");
    let info = ipc::info(&mut rt.engine, U, &q, 0).unwrap();
    assert_eq!(ipc::fill(&info, "%pumboperms_weight%"), "10");
    assert_eq!(ipc::placeholder(&info, "weight", None).as_deref(), Some("10"));
    assert_eq!(rt.pie_value(&ask("weight", Some("Steve"), None), 0).as_deref(), Some("10"));
    assert!(pie::NAMES.contains(&"weight"));
}

#[test]
fn info_without_weight_reads_as_zero() {
    // What a proxy before 1.1.0 sends through PumboBridge.
    let old =
        r#"{"name":"Steve","rank":"vip","rank_display":"VIP","prefix":"","suffix":"","groups":["vip"],"meta":{}}"#;
    let info: ipc::Info = serde_json::from_str(old).unwrap();
    assert_eq!(info.weight, 0);
    let mut rt = rt();
    rt.join(U, steve(), 0);
    install(rt);
    let reply = Rt::pie_passive_reply(&ask("weight", Some("Steve"), None), |_| Some(old.to_string()));
    assert_eq!(value(&reply).as_deref(), Some("0"));
    uninstall();
}
