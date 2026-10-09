# Using PumboPerms from your plugin

Your plugin can ask PumboPerms whether a player has a permission, read their rank, prefix and suffix, and fill in placeholders. It works on a single Pumpkin server and on a PumboProx network.

## Placeholders

| Placeholder | Shows |
| --- | --- |
| `%pumboperms_prefix%` / `%pumboperms_suffix%` | the player's prefix / suffix |
| `%pumboperms_rank%` | display name of the primary group (`VIP`) |
| `%pumboperms_group%` | name of the primary group (`vip`) |
| `%pumboperms_groups%` | every inherited group, most important first |
| `%pumboperms_meta:<key>%` | a meta value, e.g. `%pumboperms_meta:color%` |

Unknown placeholders stay in the text as they are. Prefixes keep their `&` color codes.

Available from 0.1.2-beta.

## On Pumpkin

Send JSON to the plugin `pumboperms` with Pumpkin's inter-plugin call (`ipc::send_ipc_message`). Every request has an `op`; `world` is optional and limits the answer to that world's context.

| Request | Answer |
| --- | --- |
| `{"op": "hello"}` | `{"ok": true, "plugin": "PumboPerms", "version": "…", "protocol": 1}` |
| `{"op": "check", "uuid": "…", "node": "myplugin.fly", "world": "world", "op-level": 0}` | `{"ok": true, "value": true}`, `false`, or `null` when nobody decides (use your own default). Works for offline players too. |
| `{"op": "info", "uuid": "…", "world": "world"}` | rank, display name, prefix, suffix, all groups and meta of the player |
| `{"op": "fill", "uuid": "…", "world": "world", "text": "%pumboperms_prefix%Steve"}` | `{"ok": true, "text": "&6[VIP] Steve"}` |

Errors are `{"ok": false, "error": "…"}`.

A chat, tab or scoreboard plugin usually needs only `fill`: send your format once per player and update it when you redraw.

## On PumboProx

PumboPerms on the proxy is the network's permission provider:

- **Permissions:** check them with the SDK's `permissions` interface as usual; PumboPerms answers for the whole network, with server contexts.
- **Placeholders:** they are in the proxy's placeholder registry. Put `%pumboperms_prefix%` into any text you render with `placeholders::resolve`, or into any message a proxy plugin sends with a template. Values follow the player's current server.
- **Offline players:** call the service `pumbo:permissions`, method `check-offline`.

## Moving your plugin's ranks to PumboPerms

If your plugin answers `{"op": "export-permissions"}` with data in the PumboPerms export format (the format of `/pp export`), list it in `import.plugins` in PumboPerms' `config.yml`. PumboPerms then takes the ranks over at start, and admins can use `/pp import preview|run|undo`.

Questions or ideas: [open an issue](https://github.com/PumboMC/PumboPerms/issues).
