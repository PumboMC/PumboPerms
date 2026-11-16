//! PumpkinPIE, the placeholder plugin of Pumpkin: the messages PumboPerms
//! sends it and answers, and when it registers the namespace `pumboperms`
//! (protocol 0).
//!
//! PumboPerms registers from its periodic task, never while it loads or
//! unloads and never from inside an answer to PumpkinPIE: PumpkinPIE asks
//! from inside its own message handler, and a message back would enter it
//! again. Without PumpkinPIE nothing changes; a ping every 10 seconds goes
//! unanswered.

use serde_json::{Value, json};

/// The plugin on Pumpkin that messages go to.
pub const PROVIDER: &str = "PumpkinPIE";
/// The protocol this code speaks.
pub const PROTOCOL: u64 = 0;
/// `%pumboperms_<name>%`.
pub const NAMESPACE: &str = "pumboperms";
/// The names PumpkinPIE shows for the namespace. It forwards any other name too.
pub const NAMES: [&str; 6] = ["prefix", "suffix", "rank", "group", "groups", "meta"];

/// Between pings while PumpkinPIE does not answer (ms).
pub const ABSENT_MS: u64 = 10_000;
/// Between registrations while it does (ms): a PumpkinPIE loaded again has forgotten them.
pub const REGISTERED_MS: u64 = 30_000;
/// After a refusal (ms).
pub const REFUSED_MS: u64 = 60_000;
/// After an answer in another protocol (ms).
pub const MISMATCH_MS: u64 = 600_000;

pub fn ping() -> Vec<u8> {
    br#"{"op":"ping"}"#.to_vec()
}

/// Claims the namespace; the provider asks for every value (`never` cached),
/// so a change made with `/pp` shows at once.
pub fn register() -> Vec<u8> {
    let msg = json!({ "op": "register_expansion", "namespace": NAMESPACE, "placeholders": NAMES, "cache": { "kind": "never" } });
    msg.to_string().into_bytes()
}

/// What a reply of PumpkinPIE says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// No reply: PumpkinPIE is not loaded, or the message did not reach it.
    Nothing,
    /// The answer to `ping`.
    Pong { protocol: u64, version: String },
    /// The namespace is registered.
    Registered,
    /// `ok: false`, or a reply this code cannot read: why.
    Refused(String),
}

/// Reads a reply; fields this code does not know are left alone.
pub fn heard(reply: Option<&[u8]>) -> Heard {
    let Some(bytes) = reply else { return Heard::Nothing };
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Heard::Refused("an answer that is not JSON".into());
    };
    if v.get("ok") != Some(&Value::Bool(true)) {
        let why = v.get("error").and_then(Value::as_str).unwrap_or("no reason given");
        return Heard::Refused(why.to_string());
    }
    if let Some(p) = v.get("ping") {
        let protocol = p.get("protocol").and_then(Value::as_u64).unwrap_or(u64::MAX);
        let version = p.get("version").and_then(Value::as_str).unwrap_or_default().to_string();
        return Heard::Pong { protocol, version };
    }
    if v.get("registered_expansion").is_some() {
        return Heard::Registered;
    }
    Heard::Refused("an answer without a body".into())
}

/// What to send after a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// `register_expansion`, now.
    Register,
    /// Nothing until [`PieState::due`].
    Wait,
}

/// When PumboPerms talks to PumpkinPIE next, and what it heard before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PieState {
    /// When the next ping is due (ms).
    next: u64,
    /// The namespace is registered with the PumpkinPIE that answers now.
    registered: bool,
    /// A refusal or another protocol was logged; until a registration, the next one is not.
    warned: bool,
    /// The version of PumpkinPIE from its last answer to `ping`.
    version: String,
}

impl PieState {
    pub fn due(&self, now: u64) -> bool {
        now >= self.next
    }

    /// Takes in a reply at `now`: what to send next and a line for the log
    /// (`true`: a warning). Every `ping` is answered by a registration, so
    /// one restarted PumpkinPIE gets it back within [`REGISTERED_MS`].
    pub fn step(&mut self, now: u64, heard: Heard) -> (Next, Option<(bool, String)>) {
        match heard {
            Heard::Nothing => {
                self.registered = false;
                self.next = now.saturating_add(ABSENT_MS);
                (Next::Wait, None)
            }
            Heard::Pong { protocol, version } if protocol == PROTOCOL => {
                self.version = version;
                // Kept if the registration gets no reply.
                self.next = now.saturating_add(ABSENT_MS);
                (Next::Register, None)
            }
            Heard::Pong { protocol, version } => {
                self.registered = false;
                self.next = now.saturating_add(MISMATCH_MS);
                let line = format!(
                    "PumboPerms: PumpkinPIE {version} speaks protocol {protocol}, this version speaks {PROTOCOL}; the placeholders are not registered"
                );
                (Next::Wait, self.warn_once(line))
            }
            Heard::Registered => {
                let line = (!self.registered)
                    .then(|| (false, format!("PumboPerms: placeholders registered with PumpkinPIE {}", self.version)));
                self.registered = true;
                self.warned = false;
                self.next = now.saturating_add(REGISTERED_MS);
                (Next::Wait, line)
            }
            Heard::Refused(why) => {
                self.registered = false;
                self.next = now.saturating_add(REFUSED_MS);
                (Next::Wait, self.warn_once(format!("PumboPerms: PumpkinPIE refused the placeholders: {why}")))
            }
        }
    }

    fn warn_once(&mut self, line: String) -> Option<(bool, String)> {
        let first = !self.warned;
        self.warned = true;
        first.then_some((true, line))
    }
}

/// A placeholder PumpkinPIE asks for (`on_request`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub namespace: String,
    /// What follows the namespace, in lower case (`meta`).
    pub name: String,
    /// The nickname of the online player the text is for.
    pub viewer: Option<String>,
    /// What follows `:` (`color` in `%pumboperms_meta:color%`).
    pub argument: Option<String>,
}

/// PumpkinPIE's `on_request`; `None` for any other message, which gets the
/// usual answers. A missing field or `null` is `None` or empty.
pub fn request(message: &[u8]) -> Option<Request> {
    let v: Value = serde_json::from_slice(message).ok()?;
    if v.get("op").and_then(Value::as_str) != Some("on_request") {
        return None;
    }
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    Some(Request {
        namespace: text("namespace").unwrap_or_default(),
        name: text("name").unwrap_or_default(),
        viewer: text("viewer"),
        argument: text("argument"),
    })
}

impl Request {
    /// The name and argument PumboPerms answers; `None` for another
    /// namespace or a name it does not have.
    pub fn wanted(&self) -> Option<(&'static str, Option<&str>)> {
        if !self.namespace.eq_ignore_ascii_case(NAMESPACE) {
            return None;
        }
        let name = NAMES.iter().find(|n| n.eq_ignore_ascii_case(&self.name))?;
        Some((name, self.argument.as_deref()))
    }
}

/// The reply to `on_request`. Without a value an empty body declines, and
/// PumpkinPIE leaves the placeholder in the text; it reads a bare
/// `{"ok":true}` as a broken answer.
pub fn answer(value: Option<String>) -> Vec<u8> {
    let body = value.map_or_else(|| json!({}), |v| json!({ "value": v }));
    json!({ "ok": true, "on_request": body }).to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PONG: &[u8] =
        br#"{"ok":true,"ping":{"protocol":0,"name":"PumpkinPIE","version":"0.2.5","placeholders":11,"expansions":0}}"#;
    const REGISTERED: &[u8] = br#"{"ok":true,"registered_expansion":{"namespace":"pumboperms","placeholders":["prefix","suffix","rank","group","groups","meta"],"cache":{"kind":"never"}}}"#;

    fn json(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).unwrap()
    }

    /// Registers at `now` from a fresh start: ping, pong, registration.
    fn registered_at(s: &mut PieState, now: u64) {
        assert!(s.due(now));
        assert_eq!(s.step(now, heard(Some(PONG))).0, Next::Register);
        assert_eq!(s.step(now, heard(Some(REGISTERED))).0, Next::Wait);
    }

    #[test]
    fn ping_needs_protocol_0() {
        assert_eq!(json(&ping()), json!({ "op": "ping" }));
        assert_eq!(heard(Some(PONG)), Heard::Pong { protocol: 0, version: "0.2.5".into() });
        let other = br#"{"ok":true,"ping":{"protocol":1,"name":"PumpkinPIE","version":"0.3.0"}}"#;
        let mut s = PieState::default();
        let (next, line) = s.step(0, heard(Some(other)));
        assert_eq!(next, Next::Wait);
        assert!(line.is_some_and(|(warn, l)| warn && l.contains("protocol 1")));
        // A pong without a protocol is not protocol 0.
        assert_ne!(heard(Some(br#"{"ok":true,"ping":{}}"#)), Heard::Pong { protocol: 0, version: String::new() });
    }

    #[test]
    fn register_request_names_our_placeholders() {
        let v = json(&register());
        assert_eq!(v["op"], "register_expansion");
        assert_eq!(v["namespace"], "pumboperms");
        assert_eq!(v["placeholders"], json!(["prefix", "suffix", "rank", "group", "groups", "meta"]));
        assert_eq!(v["cache"], json!({ "kind": "never" }));
    }

    #[test]
    fn register_reply_is_read() {
        assert_eq!(heard(Some(REGISTERED)), Heard::Registered);
        assert_eq!(
            heard(Some(br#"{"ok":false,"error":"'player' is reserved by the provider"}"#)),
            Heard::Refused("'player' is reserved by the provider".into())
        );
        assert_eq!(heard(Some(br#"{"ok":true}"#)), Heard::Refused("an answer without a body".into()));
        assert!(matches!(heard(Some(b"{oops")), Heard::Refused(_)));
        assert_eq!(heard(None), Heard::Nothing);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let pong = br#"{"ok":true,"extra":[1],"ping":{"protocol":0,"version":"9.9","new":true}}"#;
        assert_eq!(heard(Some(pong)), Heard::Pong { protocol: 0, version: "9.9".into() });
        let asked = br#"{"op":"on_request","namespace":"pumboperms","id":"pumboperms_rank","name":"rank","viewer":"Steve","since":"0.3"}"#;
        assert_eq!(request(asked).map(|r| r.name), Some("rank".into()));
    }

    #[test]
    fn refusal_warns_once_and_retries() {
        let mut s = PieState::default();
        let no = heard(Some(br#"{"ok":false,"error":"busy"}"#));
        let (_, line) = s.step(0, no.clone());
        assert_eq!(line, Some((true, "PumboPerms: PumpkinPIE refused the placeholders: busy".into())));
        assert!(!s.due(REFUSED_MS - 1) && s.due(REFUSED_MS));
        assert_eq!(s.step(REFUSED_MS, no).1, None, "once");
        registered_at(&mut s, 2 * REFUSED_MS);
        assert_eq!(s.step(3 * REFUSED_MS, heard(Some(br#"{"ok":false,"error":"busy"}"#))).1.map(|l| l.0), Some(true));
    }

    #[test]
    fn absent_and_null_fields_parse() {
        let bare = br#"{"op":"on_request","namespace":"pumboperms","id":"pumboperms_prefix","name":"prefix"}"#;
        let nulls = br#"{"op":"on_request","namespace":"pumboperms","id":"pumboperms_prefix","name":"prefix","viewer":null,"argument":null}"#;
        for msg in [&bare[..], &nulls[..]] {
            let r = request(msg).unwrap();
            assert_eq!((r.viewer, r.argument), (None, None));
        }
        let full = br#"{"op":"on_request","namespace":"pumboperms","id":"pumboperms_meta","name":"meta","viewer":"Steve","argument":"color"}"#;
        let r = request(full).unwrap();
        assert_eq!((r.viewer.as_deref(), r.wanted()), (Some("Steve"), Some(("meta", Some("color")))));
        assert_eq!(request(br#"{"op":"fill","uuid":"x","text":"y"}"#), None);
        assert_eq!(request(b"{oops"), None);
    }

    #[test]
    fn decline_is_an_empty_object_not_a_bare_ok() {
        assert_eq!(json(&answer(None)), json!({ "ok": true, "on_request": {} }));
        assert_eq!(json(&answer(Some("[VIP] ".into()))), json!({ "ok": true, "on_request": { "value": "[VIP] " } }));
    }

    #[test]
    fn absent_provider_is_pinged_every_10_seconds() {
        let mut s = PieState::default();
        assert!(s.due(0));
        assert_eq!(s.step(0, heard(None)), (Next::Wait, None));
        assert!(!s.due(ABSENT_MS - 1) && s.due(ABSENT_MS));
        assert_eq!(s.step(ABSENT_MS, Heard::Nothing), (Next::Wait, None));
        assert!(s.due(2 * ABSENT_MS));
    }

    #[test]
    fn registration_follows_ping() {
        let mut s = PieState::default();
        assert_eq!(s.step(5, heard(Some(PONG))), (Next::Register, None));
        let (next, line) = s.step(5, heard(Some(REGISTERED)));
        assert_eq!(
            (next, line),
            (Next::Wait, Some((false, "PumboPerms: placeholders registered with PumpkinPIE 0.2.5".into())))
        );
    }

    #[test]
    fn registration_repeats_every_30_seconds() {
        let mut s = PieState::default();
        registered_at(&mut s, 0);
        assert!(!s.due(REGISTERED_MS - 1) && s.due(REGISTERED_MS));
        assert_eq!(s.step(REGISTERED_MS, heard(Some(PONG))).0, Next::Register);
        // Registered again, without a second line in the log.
        assert_eq!(s.step(REGISTERED_MS, heard(Some(REGISTERED))), (Next::Wait, None));
        assert!(s.due(2 * REGISTERED_MS));
    }

    #[test]
    fn lost_provider_goes_back_to_ping() {
        let mut s = PieState::default();
        registered_at(&mut s, 0);
        // No answer at the next registration: a ping every 10 seconds again.
        assert_eq!(s.step(REGISTERED_MS, Heard::Nothing), (Next::Wait, None));
        let back = REGISTERED_MS + ABSENT_MS;
        assert!(!s.due(back - 1) && s.due(back));
        // Loaded again, it gets the registration, and the log says so again.
        assert_eq!(s.step(back, heard(Some(PONG))).0, Next::Register);
        assert!(s.step(back, heard(Some(REGISTERED))).1.is_some());
    }

    #[test]
    fn protocol_mismatch_waits_ten_minutes() {
        let mut s = PieState::default();
        let other = Heard::Pong { protocol: 1, version: "0.3.0".into() };
        assert!(s.step(0, other.clone()).1.is_some());
        assert!(!s.due(MISMATCH_MS - 1) && s.due(MISMATCH_MS));
        assert_eq!(s.step(MISMATCH_MS, other), (Next::Wait, None), "warned once");
    }
}
