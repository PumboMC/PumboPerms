# Commands and permissions

`/pumboperms`, or the short `/pp`, on PumboProx and on Pumpkin, from players and the console, on Pumpkin also from RCON. `/pp help` lists the commands you may use, with clicks and tooltips. Long lists (`info`, `permission info`, `parent info`, `meta info`, `groups`, `tracks`) show 20 rows per page with arrows to click; the page number goes last (`/pp group admin info 2`), and the console gets the whole list. `/pp log` shows 10 changes per page, newest first, and reads the log only back to the end of the page you open, also with `log.max-entries: 0`; `listmembers` shows 15 players. On PumboProx a reply too long for one chat message comes in several.

| Command | What it does | Permission (`pumbo.perms.`...) |
| --- | --- | --- |
| `/pp user <player> info` | Groups, permissions and meta of a player | `user.info` |
| `/pp user <player> permission info` | The player's permissions | `user.permission.info` |
| `/pp user <player> permission set\|settemp <node> [true\|false] [duration]` | Give or deny a permission | `user.permission.set` |
| `/pp user <player> permission unset\|unsettemp <node>` | Take a permission away | `user.permission.unset` |
| `/pp user <player> permission check <node>` | How a permission is decided | `user.permission.check` |
| `/pp user <player> permission clear` | Remove all of the player's permissions | `user.permission.clear` |
| `/pp user <player> parent info` | The player's groups | `user.parent.info` |
| `/pp user <player> parent add\|addtemp <group> [duration]` | Add a group | `user.parent.add` |
| `/pp user <player> parent remove\|removetemp <group>` | Remove a group | `user.parent.remove` |
| `/pp user <player> parent set <group>` | Make a group the only one | `user.parent.set` |
| `/pp user <player> meta info` | Prefixes, suffixes and meta | `user.meta.info` |
| `/pp user <player> meta set <key> <value>` / `unset <key>` | Meta values | `user.meta.set`, `user.meta.unset` |
| `/pp user <player> meta setprefix\|addprefix\|addtempprefix <priority> <text> [duration]` | Prefixes (the same with `suffix`) | `user.meta.set` |
| `/pp user <player> meta removeprefix\|removetempprefix <priority> [text]` | Remove a prefix (the same with `suffix`) | `user.meta.unset` |
| `/pp user <player> clear` | Remove everything of a player | `user.clear` |
| `/pp creategroup <name>` / `deletegroup <name>` | Create or delete a group | `group.create`, `group.delete` |
| `/pp group <name> info` | Parents, permissions and meta of a group | `group.info` |
| `/pp group <name> permission\|parent\|meta ...` | The same as for players | `group.permission.*`, `group.parent.*`, `group.meta.*` |
| `/pp group <name> rename <new name>` | Rename a group | `group.rename` |
| `/pp group <name> setweight <weight>` | Weight of a group | `group.setweight` |
| `/pp group <name> setdisplayname <name\|clear>` | Display name (the rank) | `group.setdisplayname` |
| `/pp group <name> listmembers [page]` | Players in the group | `group.listmembers` |
| `/pp groups` | All groups | `group.list` |
| `/pp createtrack <name>` / `deletetrack <name>` | Create or delete a track | `track.create`, `track.delete` |
| `/pp track <name> info` | The groups of a track | `track.info` |
| `/pp track <name> append\|insert\|remove\|clear` | Build a rank ladder | `track.edit` |
| `/pp tracks` | All tracks | `track.list` |
| `/pp promote <player> <track>` / `demote` | Move a player up or down a track | `user.promote`, `user.demote` |
| `/pp log [page]` | Recent changes | `log` |
| `/pp export [file]` / `import <file>` | Save or load everything as JSON | `export`, `import` |
| `/pp import preview\|run\|undo <source>` | Take over another source, or take it back | `import` |
| `/pp reload` / `version` | Reload the config, show the version | `reload`, `version` |

On PumboProx `/pumboperms debug` (`pumbo.perms.debug`) turns on or off a log line for every permission set PumboPerms sends to the proxy, and the import source is `file`, the proxy's `permissions.yml`.

Shorter names work too: `u`, `g` and `t` for `user`, `group` and `track`, `listgroups` and `listtracks` for `groups` and `tracks`, `info` for `version` and `?` for `help`. After a player or a group: `i` for `info`, `perm`, `p` or `permissions` for `permission`, `parents` or `inherit` for `parent`. After `permission`, `parent` or `meta`: `list` for `info`. For example `/pp u Steve p check minecraft:command.gamemode`.

Durations: `30s`, `10m`, `1h30m`, `7d`, `2w`, `1mo`, `1y`. Contexts go at the end of a command: `server=lobby`, `group=lobbies`, `world=world_nether`. Text with spaces goes in quotes: `"&6[VIP] "`; inside quotes `\"` is a quote and `\\` a backslash. A new meta key may have 1-64 characters `a-z`, `0-9`, `_`, `.` and `-`.

Server and server-group names in contexts ignore letter case: `server=survival` applies on the server `Survival` from `pumboprox.yml`. PumboProx does not start with two servers whose names differ only in letter case. A server in `pumboprox.yml` wins over a server of `managed-servers` whose name differs only in letter case; the proxy leaves the second one out and logs a warning.

The proxy does not see worlds. On PumboProx `/pp` refuses `world=`, `permission check` with `world=` adds that such entries apply nowhere, and an import keeps them and says how many there are.

`group.<name>` always answers whether the player is in that group. `/pp ... permission set group.<name>` is refused: use `parent add <name>`. An import leaves such permissions out and lists them in its reply. Behind PumboProx, PumboBridge also sets `group.<name>` on the server, so a Pumpkin plugin that checks it gets the same answer.

A player name that cannot be a Minecraft name (an offline-mode name with special spaces or `§`) shows with its UUID, and clicks and Tab completion use the UUID.

On Pumpkin the permissions are registered as `pumboperms:<name>` (the same node) and default to operators of level 3. The console may do everything. On Pumpkin, `/tp`, `/xp`, `/banip` and `/pardonip` need the permission of their command also after `/execute ... run`. Tab completion still shows these four aliases to everyone, because Pumpkin builds that list; PumboPerms refuses only running them.

## Tab completion

On both platforms Tab completes players, groups and tracks only for those with the `info` right of their kind (`pumbo.perms.user.info`, `group.info`, `track.info`). Where a permission node goes, it lists the nodes in your data, Pumpkin's commands, the nodes of the Pumbo plugins and those of PumboPerms; context keys (`server=`) come only where the command takes a context.

On PumboProx, PumboPerms answers Tab itself through the proxy's plugin suggestions: after `/pp` it offers the subcommands you may use, and on every level after that only the words you may use.

On Pumpkin, `/pp`, `/pumboperms` and every name in `commands.aliases` are separate commands with the whole tree of subcommands, so Tab works right after the space (Pumpkin 0.2.0 suggests nothing there for an alias). Above the chat box the client names the next argument (`<player>`, `<node> [<value>|<context>]`), and after a complete command it shows nothing more. Pumpkin 0.2.0 gives plugins no way to hide part of a command, so everyone who may use `/pp` sees all subcommand words. Pumpkin reads the line along that tree before PumboPerms sees it, which makes a few things work differently than on PumboProx:

- Pumpkin matches the words of the help exactly, in lower case. Any other word in their place goes to PumboPerms with the rest of the line and works as on the proxy: the shorter names above, words in upper case (`/pp USER Steve INFO`) and `editor`, which answers that the editor is not there yet. Tab offers the words of the help, and after a shorter name the words that follow it.
- Where an argument can take the word, Pumpkin gives it to the argument: after `import`, `RUN` goes to `<file>`, so `/pp import RUN proxy` fails.
- Pumpkin reads a one-word argument (`<player>`, `<group>`, `<duration>`, `[page]`...) only from `0-9 A-Z a-z _ - . +`; any other character needs quotes, `"..."` or `'...'`, and the quote has to be closed. Nodes, contexts, meta keys and values, prefixes, suffixes and display names take the rest of the line, so they need no quotes, and `"..."` keeps spaces there as on the proxy.
- Where the tree has no room for another word, Pumpkin answers with its own error instead of the PumboPerms usage line: `/pp user` without a player, or a word after a complete command (`/pp group vip create now`).
- `permission settemp`, `meta addtempprefix` and `meta addtempsuffix` show the duration in the hint as `[<duration>]`, because the node or text before it takes the rest of the line. The duration is still required; without one PumboPerms answers with the usage line.

## Export and import

`/pp export` writes `exports/<file>.json` in the data folder. It never overwrites a file and refuses to run while the database cannot be read. The export holds groups, players and tracks, without the log, the known nicknames or the import ledgers. `/pp import <file>` saves the current data first and then replaces everything.

An import fixes or leaves out entries no command could write, and its reply counts them and lists up to 20. A group or track with an invalid name comes in under a valid form of it: lower case, Polish letters without their marks, `-` for other characters, at most 36 characters, and `-2`, `-3`... when the file has that name already (`group "zła grupa!" imported as zla-grupa`). Inheritance, ranks and tracks follow the new name. The import leaves out a group with nothing usable in its name, together with every player and group that inherits it and every track it is on, so nobody keeps a rank without the denies of its groups, and it leaves out whole a track with a group the file does not have. It also leaves out players and UUIDs that are not valid, permissions that are not permissions, entries with an unknown context key, `group.<name>` set as a permission, parents without their group, prefixes, suffixes, display names and meta values over 256 bytes or with control characters, and meta keys that are empty or have spaces. It shows user names that are not Minecraft names by UUID and fixes contexts in another spelling and duplicates. A line break in a name, node or context shows as `\n` in the reply and the log. PumboPerms refuses only a file that is not a PumboPerms export.

Importing a source again keeps the tracks, inheritance, weights and ranks you set since: it takes back only what the source no longer has and adds what is new, and an imported entry you removed does not come back. `/pp import undo <source>` keeps a group that has members, is on a track or is inherited by another group. A ledger that cannot be read stops the automatic import of its source with an error in the log.

On PumboProx, players written by nickname in `permissions.yml` go to the account that has the name and joined with an online-mode (premium) account, or get their entries when such an account joins. An offline-mode account with that name gets nothing, also when it joined before the import: the import reply and the log name the player, and you give the rank by hand. The first account a nickname went to keeps it at every later import.

On PumboProx, the next start with the same `permissions.yml` imports nothing. When the file changed, the log says so; `/pp import preview file` shows what would change and `/pp import run file` imports it (with `import.follow: true` PumboPerms does it on its own). `/prox reload` hands a changed file to the running PumboPerms at once. If that fails, the log says "permissions.yml not handed to the permission provider" and the next `/prox reload` sends the file again. When PumboPerms cannot read the file it got, the log says why, and `/pp import preview|run file` answers "Cannot import file: permissions.yml has errors" with the same reason.

On Pumpkin, players a source names only by nickname get nothing, because Pumpkin does not tell whether Mojang checked an account. The import reply lists them; give them their ranks with `/pp`. From the source `proxy` only the copy PumboBridge got from the proxy since it last connected counts; an older one from disk is not imported. While the bridge has a proxy key and no such copy yet, PumboPerms waits before its first pass and Pumpkin's own defaults apply; after 10 seconds it says so in the log and keeps waiting, also when the proxy cannot be reached. Behind a proxy with PumboPerms, `/pp import run proxy` answers that there is nothing to import.

## Security

Give write rights in `/pp` only to admins you trust with everything. Any right that changes permissions is enough to become a full admin, for example:

- `pumbo.perms.user.promote` on a track that leads to the admin group: promote yourself;
- `pumbo.perms.user.permission.set`: `/pp user <you> permission set *`;
- `pumbo.perms.group.parent.add` or `user.parent.add`: put yourself into the admin group;
- `pumbo.perms.import`: `/pp import run <source>` brings back an old permission source;
- on PumboProx, `*` also grants `pumbo.proxy.bridge.key`, the PumboBridge key of the network.

`/pp import <file>` replaces the whole database, so only the console may run it (on Pumpkin also RCON, which counts as the console). `/pp` does not run from command blocks, signs or functions on Pumpkin: anyone who can place one would act as the console. Use the console or RCON for automation. `/pp log` records every change with the admin who made it.

On PumboProx, players written by nickname in `permissions.yml` get their ranks only when they join with an online-mode (premium) account. Anyone can use a nickname on an offline-mode account, so such an account gets nothing; the log names it and you give the rank by hand.

Report security problems privately, see [SECURITY.md](../SECURITY.md).
