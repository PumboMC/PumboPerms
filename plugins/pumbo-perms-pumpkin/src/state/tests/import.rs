//! Import passes, PumboBridge answers and when to ask again.

use super::*;

#[test]
fn pumpkin_import_reply_is_truthful() {
    let mut rt = rt();
    let snap = pumbo_perms_core::export::parse(
        r#"{"format":"pumboperms","version":1,"groups":[{"name":"mod"}],
                "by-name":[{"name":"Mod1","data":{"parents":[{"group":"mod"}]}}]}"#,
    )
    .unwrap();
    rt.import_pass(vec![("proxy".into(), Answer::Data("f1".into(), snap))], None, 0);
    let said = rt.logs.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
    assert!(said.contains("known only by name and get nothing here; give them their ranks with /pp: Mod1"), "{said}");
    assert!(!said.contains("premium"), "{said}");
    // Mod1 joins: nothing, and the log says why.
    rt.logs.clear();
    let plans = rt.join(U, Online { name: "Mod1".into(), ..steve() }, 1);
    rt.applied(plans);
    assert!(rt.engine.perms.user(U).is_none_or(|u| u.data.parents.is_empty()));
    assert!(rt.logs.iter().any(|(warn, l)| *warn && l.contains("Mod1 joined and got nothing")), "{:?}", rt.logs);
}

fn mode(proxy_rules: bool, fingerprint: &str, live: bool, connected: bool) -> Bridge {
    Bridge::Mode(BridgeMode { proxy_rules, fingerprint: fingerprint.into(), mode: "bridge".into(), live, connected })
}

#[test]
fn passive_flag_survives_newer_export_version() {
    let reply = br#"{"ok":true,"fingerprint":"f","data":{"format":"pumboperms","version":99,"proxy-rules":true}}"#;
    let (flag, answer) = export_answer(Some(reply));
    assert!(flag, "the flag is read even when the export is too new to import");
    assert!(matches!(answer, Answer::Bad(ref e) if e.contains("newer")), "{answer:?}");
    let m = perms_mode(br#"{"ok":true,"proxy-rules":true,"fingerprint":"f","mode":"bridge","live":true,"connected":true,"saved":1,"pending":false}"#).unwrap();
    assert_eq!(Bridge::Mode(m).proxy_rules(), Some(true));
}

#[test]
fn perms_mode_falls_back_to_export_on_old_bridge() {
    assert_eq!(perms_mode(br#"{"ok":false,"error":"unknown op perms-mode"}"#), None);
    let reply = br#"{"ok":true,"fingerprint":"f","data":{"format":"pumboperms","version":1,"proxy-rules":true}}"#;
    let (flag, answer) = export_answer(Some(reply));
    let old = Bridge::Old(flag, answer);
    assert_eq!(old.proxy_rules(), Some(true), "an older bridge decides the mode as before");
    assert!(matches!(old.proxy_source(""), Some(Answer::Data(..))));
    assert_eq!(export_answer(Some(br#"{"ok":false,"pending":true}"#)).1, Answer::Pending);
    assert_eq!(export_answer(None).1, Answer::Absent);
}

#[test]
fn tick_skips_export_when_fingerprint_same() {
    let live = mode(false, "abc", true, true);
    assert_eq!(live.proxy_source("abc"), Some(Answer::Absent), "imported already: no export asked");
    assert_eq!(live.proxy_source("old"), None, "a new table: ask for it");
    assert_eq!(Bridge::Absent.proxy_source("abc"), Some(Answer::Absent));
}

#[test]
fn stale_export_is_not_imported() {
    // From disk, the proxy not connected: neither the mode nor the data.
    let stale = mode(true, "abc", false, false);
    assert_eq!(stale.proxy_rules(), None);
    assert_eq!(stale.proxy_source(""), Some(Answer::Absent));
    // Connected, before the proxy sent this session's export: wait.
    assert_eq!(mode(true, "abc", false, true).proxy_source(""), Some(Answer::Pending));
    let mut rt = rt();
    assert!(!rt.hold(&stale, 0), "a server without a proxy goes on as a single server");
}

#[test]
fn connected_bridge_waits_for_live_answer() {
    let mut rt = rt();
    join(&mut rt, U, 0);
    let waiting = mode(true, "abc", false, true);
    assert!(rt.pass_due(0) && rt.hold(&waiting, 0));
    assert!(rt.pass_due(1_000), "asked again at the next run");
    assert!(rt.hold(&waiting, 9_000) && !rt.ready);
    assert!(rt.logs.is_empty());
    assert!(rt.hold(&waiting, HOLD_MS));
    assert!(rt.logs.iter().any(|(warn, l)| *warn && l.contains("waiting")), "{:?}", rt.logs);
    assert!(!rt.pass_due(HOLD_MS + 1_000), "then every pass only");
    rt.hold(&waiting, HOLD_MS + PASS_MS);
    assert_eq!(rt.logs.len(), 1, "warned once");
    assert!(rt.plan_all(HOLD_MS).is_empty(), "no attachments while holding");
    let live = mode(true, "abc", true, true);
    assert!(!rt.hold(&live, HOLD_MS + PASS_MS + 1));
    assert_eq!(rt.holding, None);
}

#[test]
fn import_asks_until_ready_then_only_while_waiting() {
    assert_eq!(decide_import(false, false, false, None, 0), Step::Ask);
    assert_eq!(decide_import(false, true, false, None, 0), Step::Wait);
    assert_eq!(decide_import(false, true, false, Some(10), 5), Step::Ask);
    assert_eq!(decide_import(false, true, false, Some(10), 10), Step::GiveUp);
    assert_eq!(decide_import(false, true, true, None, 0), Step::Ask);
}
