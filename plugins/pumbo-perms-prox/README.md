# PumboPerms for PumboProx

PumboPerms on the PumboProx proxy is the permission and rank system of a whole network: you set ranks and permissions in one place, on the proxy, and they work on the proxy and on every Pumpkin server behind it (through PumboBridge). It is the same PumboPerms as on Pumpkin (groups with inheritance and weights, `true`/`false` permissions, wildcards, temporary permissions, contexts, prefixes and suffixes, tracks), with `server=<name>` and `group=<server group>` as contexts.

## Features

- Everything of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#features): groups, weights, wildcards, temporary nodes, prefixes, suffixes, meta, tracks, log, JSON export and import.
- **One place for the network**: a change made with `/pp` on the proxy applies at once to proxy commands and to the servers behind it, without a restart.
- **Contexts per server**: `server=lobby` limits a permission, group or prefix to one server, `group=<name>` to a server group of `pumboprox.yml` (`server-group`).
- **Takes over `permissions.yml`** at the first start: groups, inheritance and players (also players written by nickname, who get their entries at their first join). An import only adds; conflicts go to `/pp log`; `/pp import undo file` takes it back.
- **Fails safe**: when PumboPerms is not running, the proxy goes back to `permissions.yml`.

## How it works on PumboProx

PumboPerms is the proxy's permission provider (`pumbo:permissions`). When a player joins, the proxy asks PumboPerms once for all of the player's decisions: in the global context and in the context of every server. The proxy keeps that copy and answers every permission check from it, so checks never wait for the plugin. After every change (`/pp`, an import, a temporary node ending) PumboPerms sends the new decisions of the online players (`permissions.replace`); the proxy resends the command list and PumboBridge sets the new permissions on the player's server.

On a Pumpkin server the decisions arrive as permission attachments (PumboBridge, `perm-set`): Pumpkin's command permissions (`minecraft:command.gamemode`...), every namespaced permission in the player's data, and Pumbo permissions in Pumpkin's spelling (`pumbo.bans.ban` is also sent as `pumbobans:ban`). A PumboPerms on that server steps back while the proxy's PumboPerms rules (see "PumboPerms on the servers").

Priority of sources on the proxy:

1. PumboPerms, while it runs and has loaded the player. `permissions.yml` is then not used for that player.
2. `permissions.yml`, when PumboPerms is not installed, not running (unloaded, crashed, disabled) or before it loaded the player (login gates). The proxy writes a warning to its log when it falls back.

`permissions.provider` in `pumboprox.yml`: `auto` (default) takes the installed plugin that provides permissions, `file` uses only `permissions.yml`, `pumbo-perms` names PumboPerms explicitly.

## Installation

1. Put `pumbo-perms.wasm` into the proxy's `plugins/` folder.
2. Start the proxy. It writes `plugins/pumbo-perms/config.yml`; PumboPerms keeps its data in `plugins/data/pumbo-perms/perms.redb`. Groups and players of `permissions.yml` are imported (see the log line `PumboPerms: permissions.yml: Imported file: ...`).
3. Give ranks from the console or in game:

```
/pp creategroup vip
/pp group vip parent add default
/pp group vip permission set minecraft:command.gamemode server=lobby
/pp group vip meta setprefix 10 "&6[VIP] "
/pp user Steve parent add vip
```

Updating the plugin keeps `plugins/data/pumbo-perms/` and the config; removing the `.wasm` leaves both in place (the proxy then uses `permissions.yml` again).

## Commands

`/pp` (also `/pumboperms <sub>` and `/pumbo perms <sub>`), from players and the proxy console. The subcommands and their permissions are those of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#commands); contexts are `server=<name>` and `group=<server group>` (`world=` has no meaning on the proxy). On the proxy:

| Command | Permission |
| --- | --- |
| `/pp import preview\|run\|undo file` | `pumbo.perms.import` |
| `/pumbo perms reload` (same as `/pp reload`) | `pumbo.perms.reload` |
| `/pumbo perms version` | `pumbo.perms.version` |
| `/pumbo perms debug` (log every set sent to the proxy) | `pumbo.perms.debug` |

`/pp user <player> permission check <node> server=lobby` shows the decision on a server. Tab completion lists the commands; the proxy does not complete their arguments yet.

## Taking over permissions.yml

At start the proxy sends PumboPerms its `permissions.yml` (groups with `permissions`, `inherits`, `server:` and `group:` sections, players by UUID or nickname). The first time, PumboPerms imports it:

- an import only adds: a group or player entry that is already in PumboPerms stays; a different value is a conflict, kept as it was and listed in `/pp log`;
- `server: { lobby: ... }` becomes `server=lobby`, `group: { minigames: ... }` becomes `group=minigames`, `-node` becomes `node = false`;
- players written by nickname get their entries when they join for the first time (`/pp log` notes it);
- what the file added is remembered in `plugins/data/pumbo-perms/imports/file.json`, so `/pp import undo file` takes back exactly that.

The next start with the same file does nothing. When the file changed, the log says so; `/pp import preview file` shows what it would add and `/pp import run file` imports it (with `import.follow: true` this happens on its own).

## PumboPerms on the servers

When the proxy's PumboPerms rules, a PumboPerms on a Pumpkin server behind it (with PumboBridge) sees it in the proxy's export (`proxy-rules`), takes back its own attachments and leaves them to PumboBridge; its `/pp` answers that the proxy rules. Its own data stays untouched, and when the proxy's PumboPerms goes away, it takes over again. Set ranks on the proxy with `server=<name>` instead.

## Permissions

`pumbo.perms.<action>` for every subcommand (the list is in the manifest `pumbo-perms.yml` and in the Pumpkin README), `pumbo.perms.log.notify` (messages about changes), `pumbo.perms.debug`, and `pumbo.perms.command`: whether a player sees `/pp` at all, set by PumboPerms for everyone with any `pumbo.perms` permission. `pumbo.perms.*` gives everything; the console may do everything.

## Configuration

`plugins/pumbo-perms/config.yml` (reload with `/pp reload` or `/pumbo perms reload`; the database is not reloaded). Missing options are named in the proxy log and take their defaults; a file that is not valid YAML is refused with its line and column.

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | messages (`en`, `pl` built in; `plugins/pumbo-perms/lang/<code>.yml` overrides texts) |
| `log.max-entries` | `10000` | changes kept for `/pp log` (0: unlimited) |
| `log.notify` | `true` | tell players with `pumbo.perms.log.notify` about changes |
| `import.enabled` | `true` | import `permissions.yml` at the first start |
| `import.follow` | `false` | import `permissions.yml` again whenever it changes |

## Building

```sh
./build.sh          # dist/pumbo-perms.wasm
cargo test -p pumbo-perms-core -p pumbo-perms-prox
```

Needs Rust stable with the `wasm32-wasip2` target.

## License

See the `license` field in `Cargo.toml` and the license files in the repository root.
