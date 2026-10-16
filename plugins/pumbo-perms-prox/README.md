# PumboPerms for PumboProx

PumboPerms on the PumboProx proxy is the permission and rank system of a whole network: you set ranks and permissions in one place, on the proxy, and they work on the proxy and on every Pumpkin server behind it (through PumboBridge). It is the same PumboPerms as on Pumpkin (groups with inheritance and weights, `true`/`false` permissions, wildcards, temporary permissions, contexts, prefixes and suffixes, tracks), with `server=<name>` and `group=<server group>` as contexts.

## Features

- Everything of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#features): groups, weights, wildcards, temporary nodes, prefixes, suffixes, meta, tracks, log, JSON export and import.
- **One place for the network**: a change made with `/pp` on the proxy applies at once to proxy commands and to the servers behind it, without a restart.
- **Contexts per server**: `server=lobby` limits a permission, group or prefix to one server, `group=<name>` to a server group of `pumboprox.yml` (`server-group`). Server and group names ignore letter case.
- **Takes over `permissions.yml`** at the first start: groups, inheritance and players (also players written by nickname, who get their entries when they join with an online-mode account). An import only adds; conflicts go to `/pp log`; importing the file again keeps the tracks, inheritance, weights and ranks you set since; `/pp import undo file` takes it back.
- **Keeps the ranks while it is down**: when PumboPerms restarts, is reloaded, unloaded or cannot read its database, players online keep the permissions it gave them, and the proxy refuses new logins (`on-load-failure: deny`, the default) instead of falling back to `permissions.yml`, which still holds the ranks from before the takeover.

## How it works on PumboProx

PumboPerms is the proxy's permission provider (`pumbo:permissions`). When a player joins, the proxy asks PumboPerms once for all of the player's decisions: in the global context and, for every server where they differ, in that server's context. The proxy keeps that copy and answers every permission check from it, so checks never wait for the plugin. After every change (`/pp`, an import, a temporary node ending) PumboPerms sends the new decisions of the online players the change can reach (`permissions.replace`), 25 players at a time with a pause in between; the proxy resends the command list and PumboBridge sets the new permissions on the player's server.

On a Pumpkin server the decisions arrive as permission attachments (PumboBridge, `perm-set`): Pumpkin's command permissions (`minecraft:command.gamemode`...), every namespaced permission in the player's data, `group.<name>` for every group the player inherits, and Pumbo permissions in Pumpkin's spelling (`pumbo.bans.ban` is also sent as `pumbobans:ban`, `pumbo.*` as `pumbobans:*` and the other Pumbo plugins). Pumpkin reads wildcards from the broadest one and an exact permission as a wildcard for everything below it, so a wildcard with a narrower one of another value below it is not sent (the exact permissions of the data below it are), and a catalog permission without its own decision below a permission set to true gets false. A permission set exactly also wins over a narrower wildcard: with `essentials:fly` true and `essentials:fly.*` false, Pumpkin gives true to a permission below it that the data does not name, and `permission check` shows "not sent to Pumpkin: essentials:fly hides essentials:fly.*". Update PumboBridge together with PumboPerms: an older PumboBridge does not list the Pumbo permissions. A PumboPerms on that server steps back while the proxy's PumboPerms rules (see "PumboPerms on the servers").

Priority of sources on the proxy:

1. PumboPerms, once it has loaded the player. `permissions.yml` is then not used for that player.
2. `permissions.yml`, when PumboPerms is not installed (`permissions.provider: auto` without a provider, or `file`), and before PumboPerms loaded the player (login gates).

When PumboPerms is installed but does not answer (restarting, reloaded, unloaded, crashed, disabled, its database broken), players online keep the last permissions it sent, new logins follow `on-load-failure` in `pumboprox.yml` (`deny` by default; `file-only` lets them in on `permissions.yml`), the servers behind the proxy stay in the proxy's hands, and once PumboPerms runs again the proxy loads the players online from it again. The proxy log warns once a minute and names the two fixes from the proxy console: `/prox plugins reload pumbo-perms`, or `permissions.provider: file` and a restart. With a broken database PumboPerms logs the cause once a minute.

A permission another proxy plugin sets for a player (`permissions.set`) lasts until the player leaves; a change from PumboPerms does not remove it.

`permissions.provider` in `pumboprox.yml`: `auto` (default) takes the installed plugin that provides permissions, `file` uses only `permissions.yml`, `pumbo-perms` names PumboPerms explicitly.

## Installation

1. Put `pumbo-perms.wasm` into the proxy's `plugins/` folder.
2. Start the proxy. It writes `plugins/pumbo-perms/config.yml`; PumboPerms keeps its data in `plugins/data/pumbo-perms/perms.redb`. Groups and players of `permissions.yml` are imported (see the log line `permissions.yml: PumboPerms » Imported file: ...`).
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

Put the new file in place of `plugins/pumbo-perms.wasm`, under that name (the GitHub release calls it `PumboPerms-Proxy-<version>.wasm`), and restart the proxy. `/prox plugins reload pumbo-perms` also loads the new code, but the proxy applies the manifest of the new version (its permission list, for one) only at the next restart. Keep the name: two files with the id `pumbo-perms` (an old `pumbo-perms-0.1.1.wasm` next to `pumbo-perms.wasm`) stop the proxy at start with both names and "delete the older one", because a proxy without its permission provider would give the network back to `permissions.yml`. `plugins/data/pumbo-perms/` and the config stay.

## Commands

`/pp` (also `/pumboperms <sub>`), from players and the proxy console. The subcommands and their permissions are those of [PumboPerms for Pumpkin](../pumbo-perms-pumpkin/README.md#commands); contexts are `server=<name>` and `group=<server group>`. The proxy does not see worlds: `/pp` refuses `world=`, `permission check` with `world=` adds that such entries apply nowhere, and an import keeps them and says how many there are. On the proxy:

| Command | Permission |
| --- | --- |
| `/pp import preview\|run\|undo file` | `pumbo.perms.import` |
| `/pumboperms reload` (same as `/pp reload`) | `pumbo.perms.reload` |
| `/pumboperms version` | `pumbo.perms.version` |
| `/pumboperms debug` (log every set sent to the proxy) | `pumbo.perms.debug` |

`/pp user <player> permission check <node> server=lobby` shows the decision on a server. Tab completion offers only what you may use, at every word: the subcommands your rights allow, player names, groups and tracks only with the `info` right of their kind, and where a permission goes, the nodes in the database, Pumpkin's commands, the nodes of the Pumbo plugins and those of PumboPerms. On Pumpkin, Tab shows every subcommand word to everyone who may use `/pp`.

The views `info`, `permission info`, `parent info`, `meta info`, `groups` and `tracks` show 20 rows per page with arrows to click; the page number goes last (`/pp group admin info 2`), and the console gets the whole view. A reply too long for one chat message comes in several messages.

## Taking over permissions.yml

At start the proxy sends PumboPerms its `permissions.yml` (groups with `permissions`, `inherits`, `server:` and `group:` sections, players by UUID or nickname). The first time, PumboPerms imports it:

- an import only adds: a group or player entry that is already in PumboPerms stays; a different value is a conflict, kept as it was and listed in `/pp log`;
- `server: { lobby: ... }` becomes `server=lobby`, `group: { minigames: ... }` becomes `group=minigames`, `-node` becomes `node = false`;
- players written by nickname go to the account that has the name and joined with an online-mode (premium) account, or get their entries when such an account joins (`/pp log` notes it with `nick -> UUID`). Anyone can use a nickname on an offline-mode account, so such an account gets nothing, also when it joined before the import; the entries keep waiting, the log and the import reply name the player and the admin gives the rank by hand. The first account a nickname went to keeps it at every later import, even when someone else has the name by then;
- a group whose name no command accepts comes in under a valid form of it (`zła grupa!` as `zla-grupa`), and inheritance and players follow the new name; a group with nothing usable in its name is left out together with every group and player that inherits it, and a player by nickname that is not a Minecraft name is left out. The log gives the number of such fixes and lists the first 20;
- what the file added is remembered in the database (the ledger of `file`), so `/pp import undo file` takes back exactly that.

If PumboPerms cannot read the file at all, the log says why, and `/pp import preview file` and `/pp import run file` answer with the same reason.

The next start with the same file does nothing. When the file changed, the log says so; `/pp import preview file` shows what it would add and take back, and `/pp import run file` imports it (with `import.follow: true` this happens on its own). A new import takes back only what the file no longer has and adds what is new: tracks, inheritance, weights and temporary ranks set with `/pp` stay, and an imported entry you removed with `/pp` does not come back.

`/prox reload` hands a changed `permissions.yml` to the running PumboPerms at once, which imports it when `import.follow` is `true` and otherwise says in the log that the file changed. When the handoff fails (the proxy log says `permissions.yml not handed to the permission provider`), the next `/prox reload` sends the file again, also when it did not change.

Coming from 0.1.2-beta, run `/pp export` first: the first start moves `imports/file.json` into `perms.redb` (the file is renamed to `file.json.migrated`), and 0.1.2-beta refuses the database after that. `/pp export` never overwrites a file and refuses to run while the database cannot be read; a broken database is read again once a minute.

## PumboPerms on the servers

When the proxy's PumboPerms rules, a PumboPerms on a Pumpkin server behind it (with PumboBridge) sees it in the proxy's export, takes back its own attachments and leaves them to PumboBridge; its `/pp` answers that the proxy rules. The export holds only that flag (`proxy-rules`), whatever the size of the database and whether PumboPerms on the proxy runs right now, so a server never takes over because the proxy's PumboPerms is restarting. Its own data stays untouched, and when the proxy starts without PumboPerms (the file removed, or `permissions.provider: file`), it takes over again. Set ranks on the proxy with `server=<name>` instead.

Plugins on such a server keep asking the local PumboPerms (`check`, `info`, `fill`) and get the proxy's answers: PumboBridge carries each online player's rank, prefix, suffix, groups and meta from the proxy along with the permissions.

## Permissions

`pumbo.perms.<action>` for every subcommand (the list is in the manifest `pumbo-perms.yml` and in the Pumpkin README), `pumbo.perms.log.notify` (messages about changes), `pumbo.perms.debug`, and `pumbo.perms.command`: whether a player sees `/pp` at all, set by PumboPerms for everyone with any `pumbo.perms` permission. `pumbo.perms.*` gives everything; the console may do everything.

## Configuration

`plugins/pumbo-perms/config.yml` (reload with `/pp reload` or `/pumboperms reload`; the database is not reloaded). The proxy writes it at the first start and keeps a copy of what it wrote; after an update, a file that still equals that copy follows the new version, unless a default value changed. Missing options are named in the proxy log and take their defaults; a file that is not valid YAML is refused with its line and column. Each time the file loads, PumboPerms keeps a copy in `plugins/data/pumbo-perms/config.last-good.yml`; if the file is not valid YAML at start, PumboPerms starts from that copy and logs an error, and without a copy it does not start.

| Option | Default | Meaning |
| --- | --- | --- |
| `language` | `en` | messages (`en`, `pl` built in; `plugins/pumbo-perms/lang/<code>.yml` overrides texts) |
| `log.max-entries` | `10000` | changes kept for `/pp log` (0: unlimited) |
| `log.notify` | `true` | tell players with `pumbo.perms.log.notify` about changes |
| `import.enabled` | `true` | import `permissions.yml` at the first start |
| `import.follow` | `false` | import `permissions.yml` again whenever it changes |
| `commands.aliases` | `[pp]` | short commands for `/pumboperms`, which always works; `[]`: only `/pumboperms`. A name another plugin holds is skipped with a warning. Changes after `/prox plugins reload pumbo-perms` |
| `commands.disabled` | `[]` | `/pp` subcommands to switch off (`editor`, `export`, `user`...): gone from the help and Tab completion, refused when typed. `/pumboperms <subcommand>` follows a change after `/prox plugins reload pumbo-perms` |

## Limits

Measured on an Apple M4 Pro laptop with release builds of the plugin code; a VPS is often two or three times slower, and the plugin runs as WebAssembly on the proxy, which adds to that. The proxy stops a plugin call after 200 ms of continuous work (`plugins.budget-ms`).

- A change in a group every player inherits, with 300 players online on 10 servers, 20 groups of 50 permissions and 3000 more players in the database: about 30 ms for all 300 new sets. A change that reaches one player recomputes that player alone.
- `permissions.yml` with 3000 players: about 30-45 ms for the first import with 100 players online and 15-20 ms to import it again after a change. Reading the file, the import and each group of 25 new sets are separate pieces of work for the budget. The import itself is one piece: one database transaction, about 15-30 ms of the above. In PumboProx's own test with the built `pumbo-perms.wasm` (3000 players in 20 groups of 50 permissions, then a change for 200 players online), no call went over the budget, also with `plugins.memory-mb: 64`.
- A join with 400 000 known names: up to 0.135 ms, the first after the start up to 0.57 ms (empty database cache). Known names stay in the database, and memory holds the names of the players online: with 400 000 known names, opening the database and 200 joins raised the heap peak by 0.4 MiB (159.3 MiB in 0.1.2-beta).
- Memory: with the numbers above the test process grew by about 60 MB; the proxy's default `plugins.memory-mb: 256` leaves room. On the test network the proxy held about 60 MB (footprint) through a minute of joins and changes. `plugins.memory-mb` sets the same limit for each plugin on the proxy, and 64 MB holds PumboPerms with 400 000 known names: with that limit, such a database and 3000 players in `permissions.yml`, PumboPerms opened the database file in 6 ms on the test network, and the proxy process held 197 744 KB (RSS; 141 312 KB footprint).
- Load on the test network: 4 players sending 100 commands a second each on a server and 20 `/pp` changes a second, with 3000 players in `permissions.yml`: nobody was disconnected and no plugin call ran over the budget.

## Building

```sh
./build.sh          # dist/pumbo-perms.wasm
cargo test -p pumbo-perms-core -p pumbo-perms-prox
```

Needs Rust stable with the `wasm32-wasip2` target.

## License

See the `license` field in `Cargo.toml` and the license files in the repository root.
