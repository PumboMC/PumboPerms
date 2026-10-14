# PumboPerms for PumboProx

PumboPerms on the PumboProx proxy is the permission and rank system of a whole network: you set ranks and permissions in one place, on the proxy, and they work on the proxy and on every Pumpkin server behind it (through PumboBridge). It is the same PumboPerms as on Pumpkin (groups with inheritance and weights, `true`/`false` permissions, wildcards, temporary permissions, contexts, prefixes and suffixes, tracks), with `server=<name>` and `group=<server group>` as contexts.

## Features

- Everything of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#features): groups, weights, wildcards, temporary nodes, prefixes, suffixes, meta, tracks, log, JSON export and import.
- **One place for the network**: a change made with `/pp` on the proxy applies at once to proxy commands and to the servers behind it, without a restart.
- **Contexts per server**: `server=lobby` limits a permission, group or prefix to one server, `group=<name>` to a server group of `pumboprox.yml` (`server-group`). Server and group names ignore letter case.
- **Takes over `permissions.yml`** at the first start: groups, inheritance and players (also players written by nickname, who get their entries when they join with an online-mode account). An import only adds; conflicts go to `/pp log`; importing the file again keeps the tracks, inheritance, weights and ranks you set since; `/pp import undo file` takes it back.
- **Keeps the ranks while it is down**: when PumboPerms restarts, is reloaded, unloaded or cannot read its database, players online keep the permissions it gave them, and new logins wait (`on-load-failure: deny`) instead of falling back to `permissions.yml`, which still holds the ranks from before the takeover.

## How it works on PumboProx

PumboPerms is the proxy's permission provider (`pumbo:permissions`). When a player joins, the proxy asks PumboPerms once for all of the player's decisions: in the global context and in the context of every server. The proxy keeps that copy and answers every permission check from it, so checks never wait for the plugin. After every change (`/pp`, an import, a temporary node ending) PumboPerms sends the new decisions of the online players (`permissions.replace`); the proxy resends the command list and PumboBridge sets the new permissions on the player's server.

On a Pumpkin server the decisions arrive as permission attachments (PumboBridge, `perm-set`): Pumpkin's command permissions (`minecraft:command.gamemode`...), every namespaced permission in the player's data, `group.<name>` for every group the player inherits, and Pumbo permissions in Pumpkin's spelling (`pumbo.bans.ban` is also sent as `pumbobans:ban`, `pumbo.*` as `pumbobans:*` and the other Pumbo plugins). Pumpkin reads wildcards from the broadest one and an exact permission as a wildcard for everything below it, so a wildcard with a narrower one of another value below it is not sent (the exact permissions of the data below it are), and a catalog permission without its own decision below a permission set to true gets false. A permission set exactly also wins over a narrower wildcard: with `essentials:fly` true and `essentials:fly.*` false, Pumpkin gives true to a permission below it that the data does not name, and `permission check` shows "not sent to Pumpkin: essentials:fly hides essentials:fly.*". Update PumboBridge together with PumboPerms: an older PumboBridge does not list the Pumbo permissions. A PumboPerms on that server steps back while the proxy's PumboPerms rules (see "PumboPerms on the servers").

Priority of sources on the proxy:

1. PumboPerms, once it has loaded the player. `permissions.yml` is then not used for that player.
2. `permissions.yml`, when PumboPerms is not installed (`permissions.provider: auto` without a provider, or `file`), and before PumboPerms loaded the player (login gates).

When PumboPerms is installed but does not answer (restarting, reloaded, unloaded, crashed, disabled, its database broken), players online keep the last permissions it sent, new logins follow `on-load-failure` in `pumboprox.yml` (`deny` by default; `file-only` lets them in on `permissions.yml`), the servers behind the proxy stay in the proxy's hands, and the proxy loads players without permissions again every 5 seconds. The proxy log warns once a minute and names the two fixes from the proxy console: `/prox plugins reload pumbo-perms`, or `permissions.provider: file` and a restart. With a broken database PumboPerms logs the cause once a minute.

A permission another proxy plugin sets for a player (`permissions.set`) lasts until the player leaves; a change from PumboPerms does not remove it.

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

Removing the `.wasm` leaves the data and the config in place (the proxy then uses `permissions.yml` again).

## Updating

Replace `plugins/pumbo-perms.wasm` with the new file of the same name and restart the proxy (or run `/prox plugins reload pumbo-perms`). Keep the name: two files with the id `pumbo-perms` (an old `pumbo-perms-0.1.1.wasm` next to `pumbo-perms.wasm`) stop the proxy at start with both names and "delete the older one", because a proxy without its permission provider would give the network back to `permissions.yml`. `plugins/data/pumbo-perms/` and the config stay.

## Commands

`/pp` (also `/pumboperms <sub>` and `/pumbo perms <sub>`), from players and the proxy console. The subcommands and their permissions are those of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#commands); contexts are `server=<name>` and `group=<server group>`. The proxy does not see worlds: `/pp` refuses `world=`, `permission check` with `world=` adds that such entries apply nowhere, and an import keeps them and says how many there are. On the proxy:

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
- players written by nickname go to the account that has the name and joined with an online-mode (premium) account, or get their entries when such an account joins (`/pp log` notes it with `nick -> UUID`). Anyone can use a nickname on an offline-mode account, so such an account gets nothing, also when it joined before the import; the entries keep waiting, the log and the import reply name the player and the admin gives the rank by hand. The first account a nickname went to keeps it at every later import, even when someone else has the name by then;
- what the file added is remembered in the database (the ledger of `file`), so `/pp import undo file` takes back exactly that.

The next start with the same file does nothing. When the file changed, the log says so; `/pp import preview file` shows what it would add and take back, and `/pp import run file` imports it (with `import.follow: true` this happens on its own). A new import takes back only what the file no longer has and adds what is new: tracks, inheritance, weights and temporary ranks set with `/pp` stay, and an imported entry you removed with `/pp` does not come back.

Coming from 0.1.2-beta, run `/pp export` first: the first start moves `imports/file.json` into `perms.redb` (the file is renamed to `file.json.migrated`), and 0.1.2-beta refuses the database after that. `/pp export` never overwrites a file and refuses to run while the database cannot be read; a broken database is read again once a minute.

## PumboPerms on the servers

When the proxy's PumboPerms rules, a PumboPerms on a Pumpkin server behind it (with PumboBridge) sees it in the proxy's export, takes back its own attachments and leaves them to PumboBridge; its `/pp` answers that the proxy rules. The export holds only that flag (`proxy-rules`), whatever the size of the database and whether PumboPerms on the proxy runs right now, so a server never takes over because the proxy's PumboPerms is restarting. Its own data stays untouched, and when the proxy no longer has PumboPerms in its config, it takes over again. Set ranks on the proxy with `server=<name>` instead.

Plugins on such a server keep asking the local PumboPerms (`check`, `info`, `fill`) and get the proxy's answers: PumboBridge carries each online player's rank, prefix, suffix, groups and meta from the proxy along with the permissions.

## Permissions

`pumbo.perms.<action>` for every subcommand (the list is in the manifest `pumbo-perms.yml` and in the Pumpkin README), `pumbo.perms.log.notify` (messages about changes), `pumbo.perms.debug`, and `pumbo.perms.command`: whether a player sees `/pp` at all, set by PumboPerms for everyone with any `pumbo.perms` permission. `pumbo.perms.*` gives everything; the console may do everything.

## Configuration

`plugins/pumbo-perms/config.yml` (reload with `/pp reload` or `/pumbo perms reload`; the database is not reloaded). Missing options are named in the proxy log and take their defaults; a file that is not valid YAML is refused with its line and column. Each time the file loads, PumboPerms keeps a copy in `plugins/data/pumbo-perms/config.last-good.yml`; if the file is not valid YAML at start, PumboPerms starts from that copy and logs an error, and without a copy it does not start.

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
