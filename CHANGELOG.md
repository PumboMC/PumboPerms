# Changelog

All notable changes to PumboPerms, one section per release. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.0.0] - 2026-10-16

First stable release. Before you upgrade from 0.1.2-beta, run `/pp export`: PumboPerms moves its import records into the database on the first start, and 0.1.2-beta cannot open the upgraded database. Update PumboBridge on your servers at the same time.

### Added

- Tab completion for `/pp` subcommands on PumboProx and on Pumpkin, and on PumboProx also for players, groups, tracks and permission nodes. On Pumpkin the client names the next argument above the chat box.
- `commands.aliases` and `commands.disabled` in `config.yml`: change or remove the short `/pp`, and turn off subcommands, which then leave the help and Tab completion.
- Pages of 20 rows with arrows to click in `info`, `permission info`, `parent info`, `meta info`, `groups` and `tracks`; the page number goes last (`/pp group admin info 2`). On PumboProx a reply too long for one message comes in several.
- On Pumpkin, `config.yml` and `lang/<code>.yml` files you never edited get the new options and texts with an update, and your settings keep their values. An edited file stays, and the log names what it lacks.
- `/pp user <player> info`, `permission check` and the server log show "not sent to Pumpkin" where Pumpkin would not read a wildcard or a permission the way PumboPerms decides it.
- On Pumpkin, a second copy of PumboPerms on the same data folder logs "the database is open in another instance" and refuses changes instead of writing to the same database.

### Changed

- `/pp` in chat is laid out like LuckPerms: a `[PP]` tag on every line, the help in one message grouped by topic, views between bars in the Pumbo colours, `true` in green, `false` in red and prefixes in their own colours.
- Tab offers players, groups and tracks only with the `info` right of their kind. On PumboProx it offers only the subcommands you may use; on Pumpkin everyone who may use `/pp` sees all subcommand words, because Pumpkin gives plugins no way to hide them.
- A player without a group that applies everywhere is also in `default`, on every server and in every world. The proxy and the servers count it the same way.
- On Pumpkin, `defaults.op-level` covers only `/pp`, and the permissions of other plugins follow their own operator levels. A permission without its own value below a permission set to true gets false, so a moderator with `pumbo.bans.notify` needs `pumbobans:notify.silent` on its own.
- `group.<name>` answers whether the player is in the group. `permission set group.<name>` is refused with a pointer to `parent add`, and imports leave such permissions out.
- On PumboProx, `/pp` refuses `world=`, because the proxy does not see worlds, and `permission check` with `world=` says such entries apply nowhere.
- Importing a source again keeps the tracks, inheritance, weights and ranks you set with `/pp` and takes back only what the source dropped. An imported entry you removed does not come back, and `/pp import undo` keeps groups that are still in use.
- Imports fix or leave out entries no command could write and list each fix in the reply. A group or track with an invalid name comes in under a valid form of it (`zła grupa!` as `zla-grupa`), and one bad entry no longer stops the import.
- `permission settemp` on a permission that already has an end time is refused and points to `unsettemp`. New meta keys take 1 to 64 characters from `a-z`, `0-9`, `_`, `.` and `-`, and `clear`, `info` and `meta unset` refuse extra arguments.
- `/pp reload` applies all settings and warns about the ones that need a restart. On Pumpkin, `provider.refresh-seconds` defaults to 5 instead of 1; an existing `config.yml` keeps its value.
- On PumboProx, while PumboPerms restarts or cannot read its database, players online keep their ranks and new logins follow `permissions.on-load-failure` (deny by default) instead of `permissions.yml`. `/pp` answers that PumboPerms is restarting.
- On PumboProx, a second PumboPerms file in `plugins/` stops the proxy at start with both file names, so the network does not fall back to `permissions.yml`.
- A rank change recomputes only the players it reaches, so a change in `default` on a large network stays within the proxy's time limit. The database cache takes up to 16 MiB instead of 1 GiB, and a join takes under 1 ms with 400 000 known names.

- Known player names stay in the database, and memory keeps only the players online: with 400 000 known players PumboPerms runs within 64 MB on Pumpkin and on PumboProx.
- `/pp log` reads only the page you open, also with `log.max-entries: 0`.
- The `info` answer to other plugins names a player who has not joined since the update and has no groups, permissions or meta by their UUID until they join again.

### Removed

- The file for Pumpkin 0.1.0-dev (Minecraft 26.2). Its builds are paused for now.
- `/pp editor` from the help and Tab completion. Typed, it answers that the editor is not there yet.

### Fixed

- Wildcards and denials apply on the servers as on the proxy: on Pumpkin `pumbo.*` reaches the permissions of every Pumbo plugin, and with `pumbo.bans.*` true and `pumbo.bans.exempt.*` false a moderator is no longer exempt from bans.
- Server and server group names in contexts ignore letter case: `server=survival` applies on the server `Survival`.
- Behind PumboProx, PumboPerms on a server answers other plugins with the ranks, prefixes and suffixes from the proxy, and staff who have `/pp` on the proxy have it on the servers too.
- A player's ranks reach the server at every join and server switch, also after a long resource pack download, a reconnect or a restart of the server or the proxy.
- Servers behind PumboProx stay in proxy mode with thousands of ranked players, and an old copy on disk no longer decides their mode. PumboPerms on the proxy imports a `permissions.yml` over 256 KiB.
- A change in a group that `default` inherits reaches every player, and the players a change reaches get their new command list at once.
- Temporary entries end during `/tick freeze` at the next command or join, and also when the database write fails. `parent removetemp`, `meta removetempprefix` and `meta removetempsuffix` remove only the temporary entry, and `promote` from a temporary group keeps its end time.
- On Pumpkin, a cancelled world change keeps the permissions of the old world, and a rank you remove while the player leaves does not come back at their next join.
- A `config.yml` that is not valid YAML no longer brings back the default settings: PumboPerms starts from `config.last-good.yml` and logs the error.
- `/pp export` never overwrites a file and refuses to run while the database cannot be read, so a broken database cannot replace your backup with an empty one.
- PumboPerms tries an unreadable database again every minute and logs the reason, and commands that change nothing no longer read it. An unreadable import record stops the automatic import of its source instead of starting over.
- On PumboProx, `%pumboperms_prefix%`, `%pumboperms_suffix%` and `%pumboperms_rank%` close their colours, so `&c[Admin]` does not colour the rest of the chat line.
- On PumboProx, permissions that other plugins set for a session survive a rank change, and checks for offline players see rank changes at once.
- Polish messages without English parts.
- A damaged player name record no longer blocks the database: PumboPerms skips it and logs one warning.

### Security

- On PumboProx, only the proxy itself can hand PumboPerms a `permissions.yml` or read its whole database. Other plugins get a refusal.
- `/pp import <file>` replaces the whole database, so only the console may run it (on Pumpkin also RCON). On Pumpkin, command blocks, signs and functions can no longer run `/pp` with the rights of the console.
- On PumboProx, a player that `permissions.yml` names by nickname gets the ranks only with an online-mode (premium) account, also when an offline-mode account with that name joined before the import. On Pumpkin, which does not tell whether Mojang checked an account, players a source names only by nickname get nothing, and the import reply lists them.
- On Pumpkin, `/tp`, `/xp`, `/banip` and `/pardonip` need the permission of their command after `/execute ... run` too.
- Replies about a player show the UUID next to the name. A name that cannot be a Minecraft name shows with its UUID, and clicks and Tab completion insert the UUID.
- Commands refuse prefixes, suffixes, display names and meta with control characters or over 256 bytes, and imports leave them out. A line break in an imported name shows as `\n`, so a file cannot add fake lines to the log.

## [0.1.2-beta] - 2026-10-09

### Added

- Placeholders for other plugins: `%pumboperms_prefix%`, `_suffix`, `_rank`, `_group`, `_groups` and `%pumboperms_meta:<key>%`, on Pumpkin and on PumboProx. See [INTEGRATION.md](INTEGRATION.md).

## [0.1.1-beta] - 2026-10-09

No changes in PumboPerms; built with the shared library of 0.1.1-beta.

## [0.1.0-beta.1] - 2026-10-09

First public beta.

[1.0.0]: https://github.com/PumboMC/PumboPerms/compare/v0.1.2-beta...v1.0.0
[0.1.2-beta]: https://github.com/PumboMC/PumboPerms/compare/v0.1.1-beta...v0.1.2-beta
[0.1.1-beta]: https://github.com/PumboMC/PumboPerms/compare/v0.1.0-beta.1...v0.1.1-beta
[0.1.0-beta.1]: https://github.com/PumboMC/PumboPerms/releases/tag/v0.1.0-beta.1
