# PumboPerms

PumboPerms is a permission and rank system for the [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) Minecraft server: groups with inheritance and weights, permissions with `true`/`false`, wildcards, temporary permissions and groups, contexts (server, server group, world), prefixes and suffixes, and rank ladders with promote and demote. It is written in Rust and runs as a WebAssembly plugin. The rules live in `pumbo-perms-core`, so the same data will work on the PumboProx proxy later.

## Features

- **Groups** with inheritance (a group can have parent groups) and a **weight**: when two groups disagree, the heavier one wins. A player without any group is in `default`.
- **Permissions** set to `true` or `false` on players and groups, in Pumpkin's spelling (`minecraft:command.gamemode`) or the Pumbo one (`pumbo.bans.ban`, the same node as `pumbobans:ban`).
- **Wildcards**: `pumbo.bans.*`, `minecraft:command.*`, `*`. The most specific node always wins: an exact node beats `pumbo.bans.*`, which beats `pumbo.*`, which beats `*`.
- **Temporary** permissions, groups, prefixes and suffixes (`1h30m`, `7d`); they end on their own, also while the player is online.
- **Contexts**: `server=<name>`, `group=<server group>`, `world=<name>` limit a permission, a group or a prefix to where the player is.
- **Prefixes, suffixes and meta** with priorities; the rank is the heaviest group of the player (with a display name).
- **Tracks**: ordered groups for `/pp promote` and `/pp demote`.
- **Log** of every change, notifications for staff, **JSON export and import**.
- **Tab completion** for players, groups, tracks and nodes.
- **Other plugins** can ask PumboPerms over Pumpkin's inter-plugin messages (also about offline players).
- English and Polish messages, the shared Pumbo look (prefix, colours, help pages with clicks and tooltips).

## How it works on Pumpkin

Pumpkin keeps a list of permission values per player ("attachments") and checks it before its own defaults. PumboPerms decides every permission it knows about for each online player and writes the result into that list: Pumpkin's built-in command permissions, every permission that appears in a group or user, and its own permissions. The list is updated when a player joins, after every change, when the player changes worlds (if world contexts are used), when something temporary ends (checked every second) and when an operator level changes. After a change that affects commands, the player gets the command list again, so Tab completion shows exactly what they may use.

Pumpkin never has to call PumboPerms while it checks a permission, which keeps the server safe under load.

What this cannot do: a permission that nobody named anywhere (not built into Pumpkin, not in any group or user) is decided by Pumpkin alone. Wildcards inside a namespace (`minecraft:*`, `someplugin:*`) still reach such permissions, but Pumpkin reads them from the broadest one. Pumbo plugins can ask PumboPerms directly (below), which always gives the full answer.

`provider.check-event: true` makes PumboPerms also answer Pumpkin's permission check event. **Do not use it on Pumpkin 0.2.0 or 0.1.0-dev**: there the server hangs when a plugin is hot-reloaded while players are online, and under a few hundred commands per second (every permission check then waits for the plugin, and Pumpkin runs out of threads). It is there for Pumpkin versions that fix this.

Pumpkin 0.2.0 and 0.1.0-dev run `/tp`, `/xp`, `/banip` and `/pardonip` without any permission check (fixed upstream after 0.2.0, Pumpkin #3801). PumboPerms checks these four against the permission of their command (`minecraft:command.teleport`, `.experience`, `.banip`, `.pardonip`) and answers a player without it like an unknown command, as vanilla does (not with `provider.check-event: true`).

## Compatibility

| File | Pumpkin | Minecraft |
| --- | --- | --- |
| `PumboPerms-26.3.wasm` | release `0.2.0+26.3-26.51` | 26.3 |
| `PumboPerms-26.2.wasm` | release `0.1.0-dev+26.2-26.45` | 26.2 |

Pumpkin checks the plugin API strictly: a file only loads on the server version it was built for.

## Installation

1. Put the matching `.wasm` file into the `plugins/` folder of the server.
2. Start the server. PumboPerms writes `plugins/data/pumboperms/config.yml`, `lang/en.yml` and `lang/pl.yml`, and keeps its data in `plugins/data/pumboperms/perms.redb`.
3. From the console (or as an operator of level 3), create groups and give them permissions:

```
/pp creategroup vip
/pp group vip parent add default
/pp group vip setweight 10
/pp group vip permission set minecraft:command.gamemode
/pp group vip meta setprefix 10 "&6[VIP] "
/pp user Steve parent add vip
```

The plugin needs the `fs.read.data` and `fs.write.data` permissions of Pumpkin's plugin sandbox and nothing else (no network).

## Building

```sh
./build.sh          # dist/PumboPerms-26.3.wasm and dist/PumboPerms-26.2.wasm
cargo test -p pumbo-perms-core -p pumbo-perms-pumpkin
```

Rust stable with the `wasm32-wasip2` target is required.

## Commands

`/pumboperms` with the alias `/pp`. `/pp`, `/pp help [page]`, `/pp user <player>`, `/pp group <name>` and `/pp track <name>` list the commands you may use, with clicks and tooltips.

| Command | Permission (`pumbo.perms.`…) |
| --- | --- |
| `/pp user <player> info` | `user.info` |
| `/pp user <player> permission info` | `user.permission.info` |
| `/pp user <player> permission set <node> [true\|false] [context...]` | `user.permission.set` |
| `/pp user <player> permission settemp <node> [true\|false] <duration> [context...]` | `user.permission.set` |
| `/pp user <player> permission unset <node> [context...]` | `user.permission.unset` |
| `/pp user <player> permission unsettemp <node> [context...]` | `user.permission.unset` |
| `/pp user <player> permission check <node> [context...]` | `user.permission.check` |
| `/pp user <player> permission clear [context...]` | `user.permission.clear` |
| `/pp user <player> parent info` | `user.parent.info` |
| `/pp user <player> parent add <group> [context...]` | `user.parent.add` |
| `/pp user <player> parent addtemp <group> <duration> [context...]` | `user.parent.add` |
| `/pp user <player> parent remove <group> [context...]` | `user.parent.remove` |
| `/pp user <player> parent set <group> [context...]` | `user.parent.set` |
| `/pp user <player> meta info` | `user.meta.info` |
| `/pp user <player> meta set <key> <value> [context...]` | `user.meta.set` |
| `/pp user <player> meta unset <key> [context...]` | `user.meta.unset` |
| `/pp user <player> meta setprefix\|addprefix <priority> <text> [context...]` | `user.meta.set` |
| `/pp user <player> meta addtempprefix <priority> <text> <duration> [context...]` | `user.meta.set` |
| `/pp user <player> meta removeprefix <priority> [text] [context...]` | `user.meta.unset` |
| (the same with `suffix`) | |
| `/pp user <player> promote <track> [context...]`, `/pp promote <player> <track>` | `user.promote` |
| `/pp user <player> demote <track> [context...]`, `/pp demote <player> <track>` | `user.demote` |
| `/pp user <player> clear` | `user.clear` |
| `/pp group <name> create`, `/pp creategroup <name>` | `group.create` |
| `/pp group <name> delete`, `/pp deletegroup <name>` | `group.delete` |
| `/pp group <name> info` | `group.info` |
| `/pp group <name> permission\|parent\|meta ...` (as for users) | `group.permission.*`, `group.parent.*`, `group.meta.*` |
| `/pp group <name> rename <new-name>` | `group.rename` |
| `/pp group <name> setweight <weight>` | `group.setweight` |
| `/pp group <name> setdisplayname <name\|clear>` | `group.setdisplayname` |
| `/pp group <name> listmembers [page]` | `group.listmembers` |
| `/pp groups` | `group.list` |
| `/pp track <name> create\|delete\|info` | `track.create`, `track.delete`, `track.info` |
| `/pp track <name> append <group>`, `insert <group> <position>`, `remove <group>`, `clear` | `track.edit` |
| `/pp tracks` | `track.list` |
| `/pp log [page]` | `log` |
| `/pp export [file]` | `export` |
| `/pp import <file>` | `import` |
| `/pp import run\|preview\|undo <source>` | `import` |
| `/pp reload` | `reload` |
| `/pp version` | `version` |
| `/pp editor` (planned) | `editor` |

Durations: `30s`, `10m`, `1h30m`, `7d`, `2w`, `1mo`, `1y`. Contexts: `server=lobby`, `group=lobbies`, `world=world_nether`. Text with spaces goes in quotes: `"&6[VIP] "`.

Export writes `plugins/data/pumboperms/exports/<file>.json`; import reads such a file, saves the current data as `before-import-<date>.json` first and then replaces everything.

## Taking over other sources

PumboPerms is meant to be the only plugin that writes permissions on a server. At start (in its first tick, before it writes anything) it imports what other sources hold, and from then on it rules alone:

1. `proxy`: the ranks of PumboProx for this server, through PumboBridge (the proxy's `permissions.yml` with this server's contexts merged in). The bridge keeps the last copy on disk, so this works even before it reaches the proxy.
2. The plugins listed in `import.plugins` that answer `{"op":"export-permissions"}` in the PumboPerms export format.
3. Only when neither gave anything: the attachments other plugins set on online players (`attachments`), as player permissions.

An import only adds. A group or player entry that is already there stays as it is; a different value in the source is a conflict that is kept as it was and noted in `/pp log`. Weight, prefixes and suffixes are taken only where the group has none. What a source added is remembered in `plugins/data/pumboperms/imports/<source>.json`, so `/pp import undo <source>` takes back exactly that (an entry you changed since stays), and a later import of the same source replaces only its own entries. Each source is imported once; with `import.follow: true` again whenever it changes. `/pp import preview <source>` shows what would change, `/pp import run <source>` imports by hand (also a second time).

After the first import PumboPerms asks PumboBridge to release the attachments it wrote for the proxy (`release-permissions`), and writes them alone. When the plugin is unloaded, it takes its attachments back.

**When PumboPerms runs on the proxy too**, the proxy rules: its export (through PumboBridge) says `proxy-rules`, and this PumboPerms steps back within 10 seconds. It hands its attachments to PumboBridge, which writes the proxy's decisions from then on, writes nothing itself and adds a note to every `/pp` answer ("PumboPerms on the proxy manages the ranks of this network"); `/pp version` shows the mode. Its data stays as it was, and when the proxy no longer has PumboPerms it takes over again. Set ranks on the proxy instead, with `server=<name>` for this server (see `plugins/pumbo-perms-prox/README.md`).

## Permissions

The shared Pumbo name of every permission is `pumbo.perms.<permission>`; on Pumpkin it is registered as `pumboperms:<permission>` (the same node). All of them, and `pumboperms:command` (seeing `/pp` at all), default to operators of level 3 (`defaults.op-level`), and the console may do everything.

| Permission | Gives |
| --- | --- |
| the command permissions above | the command |
| `log.notify` | a message about every change made by others |
| `pumbo.perms.*` | everything |

How a permission is decided:

1. The most specific node wins: the exact node over `a.b.*` over `a.*` over `*`.
2. Between equally specific nodes, the player's own beat their groups', and a heavier group beats a lighter one (then the closer one in the inheritance, then by name).
3. On one player or group, a node limited by contexts beats a global one (server over server group over world), and a temporary one beats a permanent one.
4. Nothing set: Pumpkin decides (operator levels). For `pumbo.*` permissions nobody set, operators of level `defaults.op-level` and up get them.

Every group a player inherits also answers `group.<name>` with `true`.

## Configuration

`plugins/data/pumboperms/config.yml` (reload with `/pp reload`; the database is not reloaded). The file is written with short comments on first start; the options:

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | messages from `lang/<code>.yml` (`en`, `pl` built in) |
| `context.server` | empty | name of this server: nodes added with `server=<name>` apply only on the server with that name; empty: such nodes never apply here |
| `context.group` | empty | group of servers this one belongs to (`group=<name>` nodes), for networks |
| `defaults.op-level` | `3` | operators of this level (1-4) and up get every `pumbo.*` permission that no group or user decides; Pumpkin's own commands keep Pumpkin's operator rules; 0: off, only groups and users decide |
| `log.max-entries` | `10000` | changes kept in the log (0: unlimited) |
| `log.notify` | `true` | tell players with `pumbo.perms.log.notify` about changes |
| `provider.check-event` | `false` | also answer each of Pumpkin's permission checks (see below, needs a restart) |
| `provider.resend-commands` | `true` | send players their command list again after changes (never with `check-event: true`) |
| `provider.refresh-seconds` | `1` | how often temporary nodes and operator levels are checked |
| `import.enabled` | `true` | import other permission sources at start (see above) |
| `import.follow` | `false` | import a source again whenever it changes (otherwise once) |
| `import.plugins` | `[]` | other plugins that answer `{"op":"export-permissions"}` |

Groups, users and tracks are not in this file: they are managed with `/pp` and stored in `perms.redb`.

`provider.check-event`: PumboPerms gives every online player the permissions it decides as Pumpkin permission attachments. That is all Pumpkin needs, and Pumpkin never has to call PumboPerms while it checks a permission. With `check-event: true` PumboPerms also answers each of Pumpkin's permission checks itself (also for permissions nobody listed). Do not use it on Pumpkin 0.2.0 or 0.1.0-dev: the server hangs when a plugin registers commands while players are online (plugin hot reload) and under many checks at once (several hundred commands per second). It is meant for testing and changes only after a restart. In this mode command lists are not sent again after a change: the resend makes Pumpkin check every command through the same event, from inside a call of this plugin, and on 0.2.0 that hangs the server now and then for good.

## For other plugins

Send JSON to the plugin `pumboperms` with Pumpkin's inter-plugin call (`ipc::send_ipc_message`):

```json
{"op": "hello"}
{"op": "check", "uuid": "…", "node": "pumbo.bans.ban", "world": "world", "op-level": 0}
{"op": "info", "uuid": "…", "world": "world"}
```

`check` answers `{"ok": true, "value": true}`, `false`, or `null` when nobody decides (use your own default). It works for offline players too. `info` gives the rank (primary group and its display name), prefix, suffix, all inherited groups and meta values: what `%rank%`, `%prefix%` and `%suffix%` will show. Errors are `{"ok": false, "error": "..."}`.

## License

See the `license` field in `Cargo.toml` and the license files in the repository root.
