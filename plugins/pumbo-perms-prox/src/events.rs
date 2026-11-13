//! What the proxy calls: start, reload, commands, players, timers and services.

use pumbo_common::clock::now_ms;
use pumbo_common::id::Uuid;
use pumbo_common::store::Store;
use pumbo_perms_core::command::{self, split_args};
use pumbo_perms_core::context::Query;
use pumbo_perms_core::export::{self, Snapshot};
use pumbo_perms_core::ipc;
use pumbo_perms_core::perms::KnownName;
use pumbo_perms_core::store::PermsStore;
use pumbo_sdk::contracts::{
    CheckOffline, CheckOfflineAnswer, HOST, METHOD_CHECK_OFFLINE, METHOD_EXPORT, METHOD_FILE, METHOD_INFO, PERMISSIONS,
    PERMISSIONS_HOST_ONLY, PermissionsExport, PermissionsInfo, PermissionsInfoRequest,
};
use pumbo_sdk::{
    CallReject, CommandEvent, PermissionSet, PlaceholderRequest, PlayerId, PlayerInfo, ServiceCall, Text, log, players,
};

use crate::push::{entries, info_json};
use crate::{BROKEN_LOG_MS, PumboPerms, cbor, query_of, server_groups, uncbor, uuid_of, who_of};

impl pumbo_sdk::Plugin for PumboPerms {
    async fn init(&self) -> Result<(), String> {
        pumbo_common::panic::install(log::error);
        let opening = now_ms();
        let store = Store::open(format!("{}/perms.redb", self.data_dir)).and_then(PermsStore::new);
        log::info(&format!("PumboPerms: database opened in {} ms", now_ms().saturating_sub(opening)));
        self.start(store)
    }

    async fn on_reload(&self) -> Result<(), String> {
        self.reload().map(|_| ())
    }

    async fn on_command(&self, e: CommandEvent) {
        // `/pp <args>`, or a subcommand of `/pumboperms`.
        let short = self.with(|s| s.aliases.contains(&e.name)).unwrap_or(false);
        let mut words = if short { Vec::new() } else { vec![e.name] };
        words.extend(e.args);
        self.run(e.player, split_args(&words.join(" "))).await;
    }

    /// Tab completion of `/pp` and `/pumboperms <sub>`: what the same line
    /// gives on Pumpkin.
    async fn on_suggest(&self, player: PlayerId, name: String, args: String) -> Option<Vec<String>> {
        let who = who_of(&players::get(player)?);
        let short = self.with(|s| s.aliases.contains(&name)).unwrap_or(false);
        let line = if short { args } else { format!("{name} {args}") };
        let mut words = split_args(&line);
        if line.is_empty() || line.ends_with(' ') {
            words.push(String::new());
        }
        let online: Vec<KnownName> = players::all()
            .iter()
            .map(|p| KnownName { uuid: uuid_of(p), name: p.profile.name.clone(), online: p.online_mode })
            .collect();
        let now = now_ms();
        // Busy: nothing rather than the host's player names.
        Some(self.with(|s| command::complete(&mut s.engine, &who, &words, &online, now)).unwrap_or_default())
    }

    /// `%pumboperms_prefix%`, `_suffix`, `_rank`, `_group`, `_groups` and
    /// `%pumboperms_meta:<key>%` of the player on their current server.
    async fn on_placeholder(&self, reqs: Vec<PlaceholderRequest>) -> Vec<Option<Text>> {
        let now = now_ms();
        let mut state = self.state.borrow_mut();
        let Some(s) = state.as_mut() else {
            return vec![None; reqs.len()];
        };
        reqs.iter()
            .map(|r| {
                let uuid = s.loaded.get(&r.player?)?.0.clone();
                let q = s.engine.place(r.context.server.as_deref().unwrap_or(""), "");
                let info = ipc::info(&mut s.engine, &uuid, &q, now)?;
                let v = ipc::placeholder(&info, &r.key, r.arg.as_deref())?;
                // Codes in a prefix must not run into the rest of a chat template.
                Some(match r.key.as_str() {
                    "prefix" | "suffix" | "rank" => Text::Mini(pumbo_common::rich::mini(&v)),
                    _ => Text::Legacy(v),
                })
            })
            .collect()
    }

    async fn on_permission_load(&self, p: PlayerInfo) -> Result<PermissionSet, String> {
        let groups = server_groups();
        let (uuid, name) = (uuid_of(&p), p.profile.name.clone());
        let now = now_ms();
        let (set, joined) = self
            .with(|s| {
                if let Some(why) = &s.engine.broken {
                    return Err(format!("the database is not available: {why}"));
                }
                s.not_provider = false;
                s.engine.server_groups = groups;
                match s.engine.seen(&uuid, &name, p.online_mode, now) {
                    Ok(Some(moved)) => log::warn(&moved),
                    Ok(None) => {}
                    Err(e) => log::warn(&format!("PumboPerms: cannot save the name of {name}: {e}")),
                }
                let joined = command::on_join(&mut s.engine, &uuid, &name, p.online_mode, now);
                let set = entries(&mut s.engine, &uuid, now);
                let info = info_json(&mut s.engine, &uuid, p.context.server.as_deref().unwrap_or(""), now);
                s.loaded.insert(p.id, (uuid.clone(), set.clone(), info));
                Ok((set, joined.into_iter().map(|o| o.reply.plain()).collect::<Vec<_>>()))
            })
            .ok_or_else(|| "PumboPerms is not loaded".to_string())??;
        for l in joined.iter().flat_map(|t| t.lines()) {
            log::info(l);
        }
        Ok(PermissionSet { entries: set })
    }

    async fn on_disconnect(&self, p: PlayerId) {
        // A reconnect can load the new session before the old one goes.
        self.with(|s| match s.loaded.remove(&p) {
            Some((uuid, ..)) if !s.loaded.values().any(|l| l.0 == uuid) => s.engine.gone(&uuid),
            _ => {}
        });
    }

    async fn on_timer(&self, timer: u64) {
        let groups = server_groups();
        let now = now_ms();
        let push = self
            .with(|s| {
                if s.timer != timer {
                    return false;
                }
                if s.engine.broken.is_some() && s.broken_logged.is_none_or(|t| now >= t.saturating_add(BROKEN_LOG_MS)) {
                    s.broken_logged = Some(now);
                    if s.engine.retry_open() {
                        log::info("PumboPerms: the database can be read again; changes work again");
                    } else if let Some(why) = &s.engine.broken {
                        log::error(&format!(
                            "PumboPerms: the database is not available ({why}); logins wait for it and changes are refused"
                        ));
                    }
                }
                let (due, saved) = s.engine.expire(now);
                if let Err(e) = saved {
                    log::warn(&format!("PumboPerms: cannot save the removal of expired entries, trying again: {e}"));
                }
                due || s.engine.server_groups != groups
            })
            .unwrap_or(false);
        if push {
            self.push_all().await;
        }
    }

    async fn on_service_call(&self, c: ServiceCall) -> Result<Vec<u8>, CallReject> {
        if c.service != PERMISSIONS.name {
            return Err(CallReject::UnknownMethod);
        }
        // Another plugin could replace every rank or read the whole database.
        if PERMISSIONS_HOST_ONLY.contains(&c.method.as_str()) && c.caller != HOST {
            return Err(CallReject::Rejected("host only".into()));
        }
        match c.method.as_str() {
            METHOD_CHECK_OFFLINE => {
                let req: CheckOffline = uncbor(&c.payload)?;
                let uuid = Uuid::parse(&req.uuid).ok_or_else(|| CallReject::Rejected("bad uuid".into()))?.to_string();
                let node =
                    pumbo_perms_core::node::normalize(&req.node).map_err(|e| CallReject::Rejected(e.to_string()))?;
                let now = now_ms();
                let value = self
                    .with(|s| {
                        let q = query_of(&s.engine, &req.context);
                        s.engine.perms.check(&uuid, &node, &q, now)
                    })
                    .ok_or_else(|| CallReject::Rejected("PumboPerms is busy".into()))?;
                cbor(&CheckOfflineAnswer { value })
            }
            METHOD_FILE => {
                self.take_file(uncbor(&c.payload)?).await;
                cbor(&true)
            }
            METHOD_EXPORT => {
                // Only the flag: servers behind the proxy step back and get
                // their sets through the bridge. The whole database would not
                // fit the host's payload limit, and nothing there reads it.
                let snap = Snapshot {
                    format: export::FORMAT.into(),
                    version: export::VERSION,
                    proxy_rules: true,
                    ..Snapshot::default()
                };
                cbor(&PermissionsExport { data: serde_json::to_string(&snap).unwrap_or_default() })
            }
            METHOD_INFO => {
                let req: PermissionsInfoRequest = uncbor(&c.payload)?;
                let uuid = Uuid::parse(&req.uuid).ok_or_else(|| CallReject::Rejected("bad uuid".into()))?.to_string();
                let now = now_ms();
                let data = self
                    .with(|s| {
                        let groups = req.groups.iter().map(|g| g.to_lowercase()).collect();
                        let q = Query::at(&req.server.to_lowercase(), groups, "");
                        ipc::info(&mut s.engine, &uuid, &q, now).and_then(|i| serde_json::to_string(&i).ok())
                    })
                    .ok_or_else(|| CallReject::Rejected("PumboPerms is busy".into()))?;
                cbor(&PermissionsInfo { data })
            }
            _ => Err(CallReject::UnknownMethod),
        }
    }
}
