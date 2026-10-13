//! The WebAssembly plugin: loading, registration with Pumpkin, other plugins.

pub mod api;
mod commands;
mod events;
mod import;
mod push;
mod settings;
mod state;

use std::cell::RefCell;

use pumbo_common::clock::now_ms;
use pumbo_common::store::Store;
use pumbo_perms_core::command::ACTIONS;
use pumbo_perms_core::engine::Engine;
use pumbo_perms_core::store::PermsStore;

use api::{
    ArgumentType, Command, CommandNode, Context, EventPriority, Permission, PermissionDefault, Plugin, PluginMetadata,
    SchedulerExt, StringType,
};

use crate::{COMMAND_NODE, PLUGIN_NAME};

thread_local! {
    /// For `ipc`, which gets no server handle.
    static SERVER: RefCell<Option<api::Server>> = const { RefCell::new(None) };
}

pub struct PumboPerms;

impl Plugin for PumboPerms {
    fn new() -> Self {
        PumboPerms
    }

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: PLUGIN_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["Patryk Skoczylas".into()],
            description: "Permissions, groups and ranks (/pp)".into(),
            dependencies: vec![],
            permissions: vec![api::permissions::FS_READ_DATA.into(), api::permissions::FS_WRITE_DATA.into()],
        }
    }

    fn on_load(&self, context: Context) -> Result<(), String> {
        pumbo_common::panic::install(api::error);
        let dir = context.get_data_folder();
        let _ = std::fs::create_dir_all(format!("{dir}/exports"));
        // Before any registration: an error here leaves nothing behind (R44).
        let (cfg, mut warnings) = settings::load_config_at_start(&dir).inspect_err(|e| {
            api::error(&format!("PumboPerms: config: {e}"));
        })?;
        let (lang, more) = settings::load_lang(&dir, &cfg.language);
        warnings.extend(more);
        for w in &warnings {
            if w.fatal {
                api::error(&format!("PumboPerms: config: {w}"));
            } else {
                api::warn(&format!("PumboPerms: config: {w}"));
            }
        }
        let store = Store::open(format!("{dir}/perms.redb")).and_then(PermsStore::new);
        let engine = Engine::new(store, cfg.core(), lang);
        if let Some(why) = &engine.broken {
            api::error(&format!("PumboPerms: the database is not available, changes are refused: {why}"));
        }
        let check_event = cfg.provider.check_event;
        let refresh_ticks = u64::from(cfg.provider.refresh_seconds) * 20;
        let op_level = cfg.defaults.op_level;
        let summary = format!(
            "PumboPerms {} loaded ({}): {} groups, {} users, {} tracks",
            env!("CARGO_PKG_VERSION"),
            api::API_LABEL,
            engine.perms.groups().count(),
            engine.perms.users().filter(|u| !u.data.is_empty()).count(),
            engine.perms.tracks().count()
        );
        state::install(state::Rt::new(dir, cfg, engine, check_event));
        state::flush_logs();
        SERVER.with(|s| *s.borrow_mut() = Some(context.get_server()));

        // Nodes in Pumpkin's registry: operators of the configured level keep
        // them by Pumpkin's own rules too (a reload may find them registered).
        let default =
            if op_level == 0 { PermissionDefault::Deny } else { PermissionDefault::Op(api::level_from(op_level)) };
        let mut nodes: Vec<(String, String)> = ACTIONS
            .iter()
            .map(|a| (format!("{PLUGIN_NAME}:{a}"), format!("PumboPerms: /pp {}", a.replace('.', " "))))
            .collect();
        nodes.push((COMMAND_NODE.to_string(), "PumboPerms: see /pp (any PumboPerms permission)".into()));
        for (node, description) in nodes {
            let _ = context.register_permission(&Permission { node, description, default, children: vec![] });
        }

        // Commands first: registering a command while players are online makes
        // Pumpkin check permissions, which must not reach this plugin's check
        // handler during its own load.
        let command = Command::new(&[PLUGIN_NAME.into(), "pp".into()], "PumboPerms: permissions, groups and ranks")
            .then(
                CommandNode::argument("args", &ArgumentType::String(StringType::Greedy))
                    .suggest(commands::Suggest)
                    .execute(commands::Pp { with_args: true }),
            )
            .execute(commands::Pp { with_args: false });
        context.register_command(command, COMMAND_NODE);

        context.register_event_handler::<api::PlayerJoinEvent, _>(events::Join, EventPriority::Lowest, false)?;
        context.register_event_handler::<api::PlayerLeaveEvent, _>(events::Leave, EventPriority::Lowest, false)?;
        context.register_event_handler::<api::PlayerChangeWorldEvent, _>(
            events::ChangeWorld,
            EventPriority::Lowest,
            false,
        )?;
        context.schedule_repeating_task(refresh_ticks, refresh_ticks, |server| events::tick(&server));
        // The first run imports other sources and takes over the attachments
        // (not in on_load: other plugins answer only after their load). Every
        // second, the pass itself decides when it is due.
        context.schedule_repeating_task(1, 20, |server| import::tick(&server));

        // Players already online (a reload of the plugin): the first import
        // pass checks the whole catalog on them, so attachments an earlier
        // run left go (R32).
        let server = context.get_server();
        for player in server.get_all_players() {
            events::track(&server, &player);
        }

        context.register_event_handler::<api::PlayerCommandSendEvent, _>(
            events::AliasGuard { check_event },
            EventPriority::Highest,
            true,
        )?;
        if check_event {
            context.register_event_handler::<api::PlayerPermissionCheckEvent, _>(
                events::Check,
                EventPriority::Highest,
                true,
            )?;
            api::warn(
                "PumboPerms: provider.check-event is on. Pumpkin 0.2.0 can hang with it (plugin hot reload with players online, many permission checks at once); use it only for testing. Command lists are not resent after changes in this mode.",
            );
        }
        api::info(&summary);
        Ok(())
    }

    fn on_unload(&self, context: Context) -> Result<(), String> {
        // Take back what was set (unset does not call back into plugins);
        // command lists are not resent here.
        let pushed = state::with(|rt| std::mem::take(&mut rt.pushed)).unwrap_or_default();
        let server = context.get_server();
        for (uuid, nodes) in pushed {
            if let Some(player) = api::parse_uuid(&uuid).and_then(|id| server.get_player_by_uuid(id)) {
                for node in nodes.keys() {
                    player.unset_permission(node);
                }
            }
        }
        // Closing the database lets a new instance open it (plugin reload).
        state::uninstall();
        SERVER.with(|s| *s.borrow_mut() = None);
        Ok(())
    }

    fn handle_ipc_message(&self, _sender: String, message: Vec<u8>) -> Result<Vec<u8>, String> {
        let now = now_ms();
        SERVER.with(|s| {
            let server = s.borrow();
            // Every entry looks at expired nodes: the periodic task may be frozen.
            if let Some(server) = server.as_ref() {
                let result = state::with(|rt| (rt.due_expiry(now), rt.resend()));
                state::flush_logs();
                if let Some((plans, resend)) = result {
                    push::apply_plans(server, None, plans, resend);
                }
            }
            if state::passive() {
                // From what the proxy decided, without the state, so `hello` from PumboBridge never finds it busy.
                let has = |uuid: &str, node: &str| state::proxy_decision(&from_bridge(uuid)?, node);
                return Ok(state::passive_reply(&message, has, player_info));
            }
            state::with(|rt| state::ipc_reply(&message, Some(&mut rt.engine), now))
                .unwrap_or_else(|| state::ipc_reply(&message, None, now))
        })
    }
}

/// PumboBridge's `player-info` answer for a player.
fn from_bridge(uuid: &str) -> Option<Vec<u8>> {
    let msg = serde_json::json!({"op": "player-info", "uuid": uuid}).to_string();
    import::send(import::BRIDGE, msg.as_bytes())
}

/// What PumboBridge carries from the proxy for a player (`perm-set.info`).
fn player_info(uuid: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&from_bridge(uuid)?).ok()?;
    Some(v.get("data")?.as_str()?.to_string())
}

crate::papi::register_plugin!(PumboPerms);
