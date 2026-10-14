# PumboPerms

PumboPerms is a permission and rank system for the [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) Minecraft server: groups with inheritance and weights, permissions with `true`/`false`, wildcards, temporary permissions and groups, contexts (server, server group, world), prefixes and suffixes, and rank ladders with promote and demote. It is written in Rust and runs as a WebAssembly plugin. The rules live in `pumbo-perms-core`, so the same data will work on the PumboProx proxy later.

## Features

- **Groups** with inheritance (a group can have parent groups) and a **weight**: when two groups disagree, the heavier one wins. A player without any group is in `default`.
- **Permissions** set to `true` or `false` on players and groups, in Pumpkin's spelling (`minecraft:command.gamemode`) or the Pumbo one (`pumbo.bans.ban`, the same node as `pumbobans:ban`).
- **Wildcards**: `pumbo.bans.*`, `minecraft:command.*`, `*`. In PumboPerms an exact node beats `pumbo.bans.*`, which beats `pumbo.*`, which beats `*`; how Pumpkin gets them is below.
- **Temporary** permissions, groups, prefixes and suffixes (`1h30m`, `7d`); they end on their own, also while the player is online.
- **Contexts**: `server=<name>`, `group=<server group>`, `world=<name>` limit a permission, a group or a prefix to where the player is.
- **Prefixes, suffixes and meta** with priorities; the rank is the heaviest group of the player (with a display name).
- **Tracks**: ordered groups for `/pp promote` and `/pp demote`.
- **Log** of every change, notifications for staff, **JSON export and import**.
- **Tab completion** for players, groups, tracks and nodes.
- **Other plugins** can ask PumboPerms over Pumpkin's inter-plugin messages (also about offline players).
- English and Polish messages, the shared Pumbo look (prefix, colours, help pages with clicks and tooltips).

## How it works on Pumpkin

Pumpkin keeps a list of permission values per player ("attachments") and checks it before its own defaults. PumboPerms decides every permission it knows about for each online player and writes the result into that list: Pumpkin's built-in command permissions, every permission that appears in a group or user, and its own permissions. The list is updated when a player joins, after every change, when the player changes worlds (if world contexts are used), when something temporary ends (checked every second) and when an operator level changes. After a change that affects commands, the player gets the command list again, so Tab completion shows exactly what they may use. When a player leaves, PumboPerms takes everything it set off them (Pumpkin would keep it until the server stops), and when they join again it checks the whole list, so a rank taken away meanwhile is gone.

`/tick freeze` also stops the check of temporary permissions every second. An expired one still goes at the next moment PumboPerms runs: a player joins, leaves, changes worlds or sends any command, or another plugin asks PumboPerms.

One copy of PumboPerms owns `perms.redb`. It keeps `perms.redb.lock` next to it and refreshes it every 10 seconds while it runs; a second copy loaded on the same folder (a second `.wasm` file, a reload that starts the new copy first) finds the lock, logs "the database is open in another instance", refuses changes and tries again every 10 seconds. A lock not refreshed for 30 seconds counts as left behind. A server that crashed or was killed leaves its lock behind, so after a quick restart PumboPerms waits up to 30 seconds for it to go stale: until then it imports nothing, sets no permissions and refuses changes, then opens the database and sets the permissions of the players online. `/tick freeze` stops the periodic refresh, so a freeze longer than 30 seconds in which nobody uses a command or joins, together with loading a second copy in that time, lets both copies open the database.

If PumboPerms itself crashes (Pumpkin then shows the plugin as failed), the permissions it last set stay on online players until the server restarts, temporary ones included. Restart the server; `/plugin unload` does not take them off.

Pumpkin never has to call PumboPerms while it checks a permission, which keeps the server safe under load.

The list also holds every permission of the Pumbo plugins (`pumbobans:ban`, `pumbofilter:bypass` and so on), so each gets its own value even when the data only has a wildcard. Pumpkin reads wildcards from the broadest one, and it reads a permission set exactly as a wildcard for everything below it. So PumboPerms sends a player's list in a form Pumpkin reads the same way:

- A wildcard with a narrower wildcard of another value below it is not sent (`pumbobans:*` true with `pumbobans:exempt.*` false: only the narrower one goes). The server log names it once per player and change, and `/pp user <name> info` and `permission check` show "not sent to Pumpkin: pumbobans:* hides pumbobans:exempt.*".
- A permission set exactly also wins over a narrower wildcard below it. With `essentials:fly` true and `essentials:fly.*` false, a permission below it that no group or user names gets true from Pumpkin. The log and `/pp` show "not sent to Pumpkin: essentials:fly hides essentials:fly.*"; give such a permission its own value.
- A permission from the list without its own decision, below a permission set to true, gets false. A moderator with `pumbo.bans.notify` does not get `pumbobans:notify.silent` from Pumpkin's operator level; give it to them on its own.

What this cannot do: a permission that nobody named anywhere (not built into Pumpkin, not in any group or user, not a Pumbo permission) is decided by Pumpkin alone. Wildcards inside a namespace (`minecraft:*`, `someplugin:*`) still reach such permissions, but not a wildcard that was left out as above: there Pumpkin's own default decides, so an operator may get such a permission even when PumboPerms says no. Pumbo plugins can ask PumboPerms directly (below), which always gives the full answer.

`provider.check-event: true` makes PumboPerms also answer Pumpkin's permission check event. **Do not use it on Pumpkin 0.2.0 or 0.1.0-dev**: there the server hangs when a plugin is hot-reloaded while players are online, and under a few hundred commands per second (every permission check then waits for the plugin, and Pumpkin runs out of threads). It is there for Pumpkin versions that fix this.

Pumpkin 0.2.0 and 0.1.0-dev run `/tp`, `/xp`, `/banip` and `/pardonip` without any permission check (fixed upstream after 0.2.0, Pumpkin #3801), also after `/execute ... run`. PumboPerms checks these four, typed directly or after any `run` of `/execute`, against the permission of their command (`minecraft:command.teleport`, `.experience`, `.banip`, `.pardonip`) and answers a player without it like an unknown command, as vanilla does. Tab completion still shows the four aliases to everyone, because Pumpkin builds that list; only running them is refused.

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

## Updating

Replace the file with the new one of the same name (`PumboPerms-26.3.wasm` or `PumboPerms-26.2.wasm`) and restart the server. Keep the name, so the update overwrites the old file instead of adding a second PumboPerms next to it. `plugins/data/pumboperms/` stays.

Coming from 0.1.2-beta, run `/pp export` first. The first start of the new version moves the import ledgers (`imports/<source>.json`, renamed to `.json.migrated`) into `perms.redb` and raises its schema to 2; 0.1.2-beta then refuses that file as newer than it understands. To go back, put the old file in place and import the export with `/pp import <file>` on a fresh `perms.redb`.

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

`/pp` runs for players, the console and RCON. Command blocks, signs and functions get a refusal and change nothing (anyone who can place one would otherwise act as the console); use the console or RCON for automation. `/pp import <file>` replaces the whole database, so only the console may run it.

Durations: `30s`, `10m`, `1h30m`, `7d`, `2w`, `1mo`, `1y`. Contexts: `server=lobby`, `group=lobbies`, `world=world_nether`. Text with spaces goes in quotes: `"&6[VIP] "`.

Export writes `plugins/data/pumboperms/exports/<file>.json`. It never overwrites a file (choose another name) and refuses to run while the database cannot be read. The export holds groups, players and tracks, not the log, the known nicknames or the import ledgers. Import reads such a file, saves the current data as `before-import-<date and time with milliseconds>.json` first and then replaces everything.

## Taking over other sources

PumboPerms is meant to be the only plugin that writes permissions on a server. At start (in its first tick, before it writes anything) it imports what other sources hold, and from then on it rules alone:

1. `proxy`: the ranks of PumboProx for this server, through PumboBridge (the proxy's `permissions.yml` with this server's contexts merged in). Only the copy the bridge got from the proxy since it last connected counts; an older one from disk is not imported. While the bridge has a proxy key and no such copy yet, PumboPerms waits before its first pass and Pumpkin's own defaults apply; after 10 seconds it says so in the log and keeps waiting, also when the proxy cannot be reached.
2. The plugins listed in `import.plugins` that answer `{"op":"export-permissions"}` in the PumboPerms export format.
3. Only when neither gave anything: the attachments other plugins set on online players (`attachments`), as player permissions.

An import only adds. A group or player entry that is already there stays as it is; a different value in the source is a conflict that is kept as it was and noted in `/pp log`. Weight, prefixes and suffixes are taken only where the group has none. What a source added is remembered in the database (its ledger), so `/pp import undo <source>` takes back exactly that (an entry you changed since stays, and so does a group that is on a track, inherited by another group or has members). A later import of the same source takes back only what the source no longer has and adds what is new; an imported entry you removed in PumboPerms stays removed. Players the source names only by nickname get nothing here, because Pumpkin does not tell whether Mojang checked an account: the import reply lists them, give them their ranks with `/pp`. A ledger that cannot be read stops the automatic import of that source with an error in the log. Each source is imported once; with `import.follow: true` again whenever it changes. `/pp import preview <source>` shows what would change, `/pp import run <source>` imports by hand (also a second time).

After the first import PumboPerms asks PumboBridge to release the attachments it wrote for the proxy (`release-permissions`), and writes them alone. When the plugin is unloaded, it takes its attachments back.

**When PumboPerms runs on the proxy too**, the proxy rules: its export (through PumboBridge) says `proxy-rules` and nothing else, and this PumboPerms steps back within 10 seconds; `/pp import run proxy` answers that there is nothing to import. It hands its attachments to PumboBridge, which writes the proxy's decisions from then on, writes nothing itself and adds a note to every `/pp` answer ("PumboPerms on the proxy manages the ranks of this network"); `/pp version` shows the mode. Its data stays as it was, and when the proxy no longer has PumboPerms it takes over again. Set ranks on the proxy instead, with `server=<name>` for this server (see `plugins/pumbo-perms-prox/README.md`).

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
4. Nothing set: Pumpkin decides (operator levels, and each plugin registers its own). For PumboPerms permissions nobody set, operators of level `defaults.op-level` and up may use `/pp`.

A player without a group that applies everywhere (no context) is also in `default`, on every server and in every world: a player whose only group is `vip` in `server=survival` has `vip` and `default` there, and `default` elsewhere.

Every group a player inherits also answers `group.<name>` with `true`, whatever is set by hand. `permission set group.<name>` is refused: use `parent add <name>`, and an import leaves such permissions out with a line in its reply.

## Configuration

`plugins/data/pumboperms/config.yml` (reload with `/pp reload`; the database is not reloaded). The file is written with short comments on first start. Each time it loads, PumboPerms keeps a copy as `config.last-good.yml`; if the file is not valid YAML at start, PumboPerms starts from that copy and logs an error, and without a copy it does not start. The options:

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | messages from `lang/<code>.yml` (`en`, `pl` built in) |
| `context.server` | empty | name of this server: nodes added with `server=<name>` apply only on the server with that name; empty: such nodes never apply here |
| `context.group` | empty | group of servers this one belongs to (`group=<name>` nodes), for networks |
| `defaults.op-level` | `3` | operators of this level (1-4) and up may use every `/pp` subcommand that no group or user decides; other plugins and Pumpkin's own commands keep their own operator rules; 0: off, only groups and users decide. Pumpkin's own default for the `pumboperms:` nodes changes only after a server restart |
| `log.max-entries` | `10000` | changes kept in the log (0: unlimited) |
| `log.notify` | `true` | tell players with `pumbo.perms.log.notify` about changes |
| `provider.check-event` | `false` | also answer each of Pumpkin's permission checks (see below, needs a restart) |
| `provider.resend-commands` | `true` | send players their command list again after changes (never with `check-event: true`) |
| `provider.refresh-seconds` | `1` | how often temporary nodes and operator levels are checked |
| `import.enabled` | `true` | import other permission sources at start (see above) |
| `import.follow` | `false` | import a source again whenever it changes (otherwise once) |
| `import.plugins` | `[]` | other plugins that answer `{"op":"export-permissions"}` |

Groups, users and tracks are not in this file: they are managed with `/pp` and stored in `perms.redb`.

After an update, an existing `lang/<code>.yml` keeps its old texts. If it has `target-user: "{0}"`, replies about a player do not show the UUID: delete that line, or write `"{0} &8({1})&r"`.

`provider.check-event`: PumboPerms gives every online player the permissions it decides as Pumpkin permission attachments. That is all Pumpkin needs, and Pumpkin never has to call PumboPerms while it checks a permission. With `check-event: true` PumboPerms also answers each of Pumpkin's permission checks itself (also for permissions nobody listed). Do not use it on Pumpkin 0.2.0 or 0.1.0-dev: the server hangs when a plugin registers commands while players are online (plugin hot reload) and under many checks at once (several hundred commands per second). It is meant for testing and changes only after a restart. In this mode command lists are not sent again after a change: the resend makes Pumpkin check every command through the same event, from inside a call of this plugin, and on 0.2.0 that hangs the server now and then for good.

## For other plugins

Send JSON to the plugin `pumboperms` with Pumpkin's inter-plugin call (`ipc::send_ipc_message`):

```json
{"op": "hello"}
{"op": "check", "uuid": "…", "node": "pumbo.bans.ban", "world": "world", "op-level": 0}
{"op": "info", "uuid": "…", "world": "world"}
{"op": "fill", "uuid": "…", "world": "world", "text": "%pumboperms_prefix%Steve"}
```

`check` answers `{"ok": true, "value": true}`, `false`, or `null` when nobody decides (use your own default). It works for offline players too. While PumboPerms on the proxy manages the network, the answers come from the proxy, in the same format: `check` reads the permissions the proxy sent for the player (`null` for a permission it did not send, whatever Pumpkin's default for it is), `info` and `fill` use what PumboBridge brings from the proxy, and `hello` has `"passive": true`. For a player who is offline, or whose data has not arrived yet, they answer `{"ok": false, "error": "proxy-rules", "passive": true}`. `info` gives the rank (primary group and its display name), prefix, suffix, all inherited groups and meta values: what `%rank%`, `%prefix%` and `%suffix%` will show. Errors are `{"ok": false, "error": "..."}`.

### Placeholders

`fill` returns the text with the placeholders filled in: `{"ok": true, "text": "&6[VIP] Steve"}`. On PumboProx the same placeholders are in the proxy's placeholder registry, for every proxy plugin.

| Placeholder | Shows |
| --- | --- |
| `%pumboperms_prefix%` / `%pumboperms_suffix%` | the player's prefix / suffix |
| `%pumboperms_rank%` | display name of the primary group |
| `%pumboperms_group%` | name of the primary group |
| `%pumboperms_groups%` | every inherited group, most important first |
| `%pumboperms_meta:<key>%` | a meta value, e.g. `%pumboperms_meta:color%` |

Unknown placeholders stay in the text as they are.

## License

See the `license` field in `Cargo.toml` and the license files in the repository root.
