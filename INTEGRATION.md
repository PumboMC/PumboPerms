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

Unknown placeholders stay in the text as they are. On Pumpkin, `fill` keeps the `&` color codes of prefixes; on PumboProx they come as MiniMessage (see below).

Available from 0.1.2-beta.

## On Pumpkin

Send JSON to the plugin `pumboperms` with Pumpkin's inter-plugin call (`ipc::send_ipc_message`). Every request has an `op`; `world` is optional and limits the answer to that world's context.

| Request | Answer |
| --- | --- |
| `{"op": "hello"}` | `{"ok": true, "plugin": "PumboPerms", "version": "…", "protocol": 1, "ops": ["hello", "check", "info", "fill"]}` |
| `{"op": "check", "uuid": "…", "node": "myplugin.fly", "world": "world", "op-level": 0}` | `{"ok": true, "value": true}`, `false`, or `null` when nobody decides (use your own default). Works for offline players too. |
| `{"op": "info", "uuid": "…", "world": "world"}` | rank, display name, prefix, suffix, all groups and meta of the player |
| `{"op": "fill", "uuid": "…", "world": "world", "text": "%pumboperms_prefix%Steve"}` | `{"ok": true, "text": "&6[VIP] Steve"}` |

Errors are `{"ok": false, "error": "…"}`. An `op` this version does not know answers `{"ok": false, "error": "unsupported op"}`, and a known `op` with missing or bad fields `{"ok": false, "error": "bad request: …"}`. Send only the ops that `hello` lists.

A chat, tab or scoreboard plugin usually needs only `fill`: send your format once per player and update it when you redraw.

The messages only read. Ranks and permissions change only through `/pp` and the console. Pumpkin passes each message with the `sender` name the calling plugin declares about itself, so PumboPerms does not treat that name as proof of who asks.

### Behind PumboProx

Behind PumboProx with PumboPerms on the proxy, ask the local PumboPerms the same way as on a single server: the answers come from the proxy, in the same format. `check` reads the permissions the proxy sent for the player: a permission the proxy did not send answers `null`, even when Pumpkin has a default for it (an operator level, for example), so the answer matches PumboPerms on a single server with no data for it; `info` and `fill` use the rank, prefix, suffix, groups and meta PumboBridge brings from the proxy with the player's permissions. `hello` adds `"passive": true`. For a player who is offline, or whose data has not arrived yet, `check`, `info` and `fill` answer `{"ok": false, "error": "proxy-rules", "passive": true}`. The `world` field is ignored there: the proxy does not see worlds.

PumboPerms learns from PumboBridge whether the proxy rules. It asks `{"op": "perms-mode"}` every 10 seconds (every second for the first 10 seconds while it waits for the proxy before its first pass) and gets `{"ok": true, "proxy-rules": …, "mode": "bridge|local", "live": …, "connected": …}`. `live` is `true` only for the export PumboBridge received from the proxy in its current session; a copy from disk or from an earlier session does not switch the mode. `connected` is `true` when the bridge has a proxy key. PumboBridge accepts the mode switches `release-permissions` and `take-permissions` only from a plugin that declares the name `pumboperms`; Pumpkin does not check that name.

### Moving your plugin's ranks to PumboPerms

If your plugin answers `{"op": "export-permissions"}` with `{"ok": true, "fingerprint": "…", "data": {…}}`, where `data` is in the PumboPerms export format (the format of `/pp export`) and `fingerprint` changes whenever the data does, list it in `import.plugins` in PumboPerms' `config.yml`. PumboPerms then takes the ranks over at start, and admins can use `/pp import preview|run|undo <your plugin>`. While your data is not ready, answer `{"ok": false, "pending": true}`: PumboPerms asks again every 10 seconds for up to 2 minutes after the start.

The import fixes or leaves out entries no command could write, such as a group or track whose name no command accepts (it comes in under a valid form of it), and lists each one in its reply. Players your data names only by nickname get nothing, because Pumpkin does not tell whether Mojang checked an account.

## On PumboProx

PumboPerms on the proxy is the network's permission provider:

- **Permissions:** check them with the SDK's `permissions` interface as usual; PumboPerms answers for the whole network, with server contexts.
- **Placeholders:** they are in the proxy's placeholder registry. Put `%pumboperms_prefix%` into any text you render with `placeholders::resolve`, or into any message a proxy plugin sends with a template. Values follow the player's current server. `%pumboperms_prefix%`, `%pumboperms_suffix%` and `%pumboperms_rank%` come as MiniMessage that closes its own tags, so `&r` inside a prefix and the end of the prefix go back to the style of your template; the other placeholders are text with `&` codes.
- **Offline players:** `permissions::has_offline` in the SDK, or the service `pumbo:permissions`, method `check-offline`.

`file`, `export` and `info` of `pumbo:permissions@1.0` are for the proxy host only. PumboProx answers any plugin that calls them with `not-declared` and never passes the call on to PumboPerms; PumboPerms also refuses them from any caller other than the host. Other plugins ask through `check-offline` or the SDK permission API (`permissions::has`).

A proxy plugin may declare only its own permission nodes under `permissions`: `pumbo.<id without "pumbo-">.`, `pumbo.<id>.` or `<id>:`. A manifest with another node, or with `default: true` on a node under `pumbo.proxy.` or `minecraft:`, fails to load and the error lists the allowed prefixes. Two plugins whose ids give the same prefix, such as `bans` and `pumbo-bans` (both `pumbo.bans.`), stop the proxy start with an error that names both. With `permissions.provider: auto` and more than one installed provider, PumboProx does not start until `services.providers` or `permissions.provider` names one.

With PumboPerms on the proxy, the export PumboBridge receives is only the flag `"proxy-rules": true` (no groups or users), whatever state PumboPerms on the proxy is in. Each permission set PumboBridge receives carries the player's `info` from the proxy (rank, prefix, suffix, groups, meta), so `info` and `fill` on the server answer like on a single server.

The topic `pumbo:rank-changed` is reserved for PumboPerms: it does not publish it yet, so do not rely on it.

Questions or ideas: [open an issue](https://github.com/PumboMC/PumboPerms/issues).
