//! Senders of `/pp` and answers to other plugins, also while the proxy rules.

use super::*;

#[test]
fn non_player_senders_get_no_console_rights() {
    assert_eq!(who_for(SenderKind::CommandBlock, None), None);
    assert_eq!(who_for(SenderKind::Dummy, None), None);
    assert_eq!(who_for(SenderKind::Player, None), None);
    assert!(who_for(SenderKind::Console, None).is_some_and(|w| w.is_console()));
    assert!(who_for(SenderKind::Rcon, None).is_some_and(|w| w.is_console()));
    let p = who_for(SenderKind::Player, Some(("Steve", U, 2, "world"))).unwrap();
    assert_eq!((p.uuid.as_deref(), p.op_level), (Some(U), 2));
    assert!(!p.is_console());
}

#[test]
fn passive_ipc_refuses_reads() {
    let mut rt = rt();
    give_vip(&mut rt);
    let check = format!(r#"{{"op":"check","uuid":"{U}","node":"group.vip"}}"#);
    let info = format!(r#"{{"op":"info","uuid":"{U}"}}"#);
    let fill = format!(r#"{{"op":"fill","uuid":"{U}","text":"%pumboperms_group%"}}"#);
    let local: serde_json::Value =
        serde_json::from_slice(&ipc_reply(check.as_bytes(), Some(&mut rt.engine), 0).unwrap()).unwrap();
    assert_eq!(local["value"], true);
    // Nothing from the proxy for this player (offline, or not arrived yet).
    for msg in [&check, &info, &fill] {
        let v: serde_json::Value =
            serde_json::from_slice(&passive_reply(msg.as_bytes(), |_, _| None, |_| None)).unwrap();
        assert_eq!(
            (v["ok"].clone(), v["error"].clone(), v["passive"].clone()),
            (false.into(), "proxy-rules".into(), true.into()),
            "{msg}"
        );
    }
    let hello: serde_json::Value =
        serde_json::from_slice(&passive_reply(br#"{"op":"hello"}"#, |_, _| None, |_| None)).unwrap();
    assert_eq!((hello["ok"].clone(), hello["passive"].clone()), (true.into(), true.into()));
    assert_eq!(hello["ops"], serde_json::json!(pumbo_perms_core::ipc::OPS));
    let other: serde_json::Value =
        serde_json::from_slice(&passive_reply(br#"{"op":"nope"}"#, |_, _| None, |_| None)).unwrap();
    assert_eq!(other["error"], "unsupported op");
    assert!(ipc_reply(check.as_bytes(), None, 0).is_err());
}

#[test]
fn passive_ipc_answers_from_proxy_data() {
    let info = pumbo_perms_core::ipc::Info {
        name: "Steve".into(),
        rank: "vip".into(),
        rank_display: "VIP".into(),
        prefix: "[VIP] ".into(),
        suffix: String::new(),
        groups: vec!["vip".into(), "default".into()],
        meta: Default::default(),
    };
    let data = serde_json::to_string(&info).unwrap();
    let bridge = serde_json::json!({
        "ok": true, "data": data, "nodes": {"pumbobans:ban": true, "minecraft:command.kick": false},
    })
    .to_string();
    let ask = |msg: String| -> serde_json::Value {
        let reply = passive_reply(
            msg.as_bytes(),
            |uuid, node| {
                assert_eq!(uuid, U);
                proxy_decision(bridge.as_bytes(), node)
            },
            |uuid| (uuid == U).then(|| data.clone()),
        );
        serde_json::from_slice(&reply).unwrap()
    };
    let v = ask(format!(r#"{{"op":"check","uuid":"{U}","node":"pumbo.bans.ban"}}"#));
    assert_eq!((v["ok"].clone(), v["value"].clone()), (true.into(), true.into()), "read in Pumpkin's spelling");
    assert!(ask(format!(r#"{{"op":"check","uuid":"{U}","node":"essentials.fly"}}"#))["value"].is_null());
    assert_eq!(ask(format!(r#"{{"op":"check","uuid":"{U}","node":"minecraft:command.kick"}}"#))["value"], false);
    // Registered in Pumpkin with an operator default, but not in the set: no decision.
    let v = ask(format!(r#"{{"op":"check","uuid":"{U}","node":"minecraft:command.gamemode"}}"#));
    assert_eq!((v["ok"].clone(), v["value"].clone()), (true.into(), serde_json::Value::Null));
    assert_eq!(proxy_decision(br#"{"ok":true,"data":null,"nodes":null}"#, "a:b"), None, "no set yet");
    assert_eq!(proxy_decision(br#"{"ok":true,"data":null}"#, "a:b"), None, "an older bridge");
    let v = ask(format!(r#"{{"op":"info","uuid":"{}"}}"#, U.replace('-', "")));
    assert_eq!(
        (v["ok"].clone(), v["rank_display"].clone(), v["prefix"].clone()),
        (true.into(), "VIP".into(), "[VIP] ".into())
    );
    let v = ask(format!(r#"{{"op":"fill","uuid":"{U}","text":"%pumboperms_prefix%Steve"}}"#));
    assert_eq!(v["text"], "[VIP] Steve");
    let other = "00000000-0000-0000-0000-000000000002";
    assert_eq!(ask(format!(r#"{{"op":"info","uuid":"{other}"}}"#))["error"], "proxy-rules");
}
