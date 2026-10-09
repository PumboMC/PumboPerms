//! The WebAssembly plugin: loading, registration with Pumpkin, other plugins.

pub mod api;
mod commands;
mod events;
mod import;
mod push;
mod settings;
mod state;

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
        let dir = context.get_data_folder();
        let _ = std::fs::create_dir_all(format!("{dir}/exports"));
        let (cfg, mut warnings) = settings::load_config(&dir);
        let (lang, more) = settings::load_lang(&dir, &cfg.language);
        warnings.extend(more);
        for w in &warnings {
            api::warn(&format!("PumboPerms: config: {w}"));
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
        // (not in on_load: other plugins answer only after their load).
        context.schedule_repeating_task(1, 200, |server| import::tick(&server));

        // Players already online (a reload of the plugin); their attachments
        // follow the first import pass.
        for player in context.get_server().get_all_players() {
            events::track(&player);
        }

        if !check_event {
            context.register_event_handler::<api::PlayerCommandSendEvent, _>(
                events::AliasGuard,
                EventPriority::Highest,
                true,
            )?;
        }
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
        Ok(())
    }

    fn handle_ipc_message(&self, _sender: String, message: Vec<u8>) -> Result<Vec<u8>, String> {
        if state::passive()
            && serde_json::from_slice::<serde_json::Value>(&message)
                .is_ok_and(|v| v.get("op").and_then(|o| o.as_str()) == Some("hello"))
        {
            // PumboBridge asks who writes the permissions: not this plugin.
            let reply = serde_json::json!({
                "ok": true, "plugin": "PumboPerms", "version": env!("CARGO_PKG_VERSION"),
                "protocol": pumbo_perms_core::ipc::PROTOCOL, "passive": true,
            });
            return Ok(reply.to_string().into_bytes());
        }
        let now = now_ms();
        state::with(|rt| pumbo_perms_core::ipc::handle(&mut rt.engine, &message, now, env!("CARGO_PKG_VERSION")))
            .ok_or_else(|| "PumboPerms is busy".to_string())
    }
}

crate::papi::register_plugin!(PumboPerms);
