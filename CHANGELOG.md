# Changelog

All notable changes to PumboPerms, one section per release. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.0.0] - 2026-10-16

First stable release. Run `/pp export` before you upgrade from 0.1.2-beta: 1.0.0 upgrades the database, and 0.1.2-beta cannot open it again.

### Added

- Tab completion for `/pp`.
- Command aliases and switching subcommands off.
- Pages in long lists.
- Command list for `/pp user`, `/pp group` and `/pp track`.
- New config and language options added on update.

### Changed

- New chat formatting.
- Tab shows only what you may use (on Pumpkin 0.1.0-dev it still lists every command).
- Players without a group are in `default` everywhere.
- `group.<name>` checks group membership.
- `defaults.op-level` covers only `/pp`.
- Imports keep your own changes when run again.
- Imports skip broken entries instead of stopping.
- `/pp reload` applies all settings.
- `provider.refresh-seconds` defaults to 5.
- Faster rank changes.
- Lower memory use with many players.
- `/pp log` reads one page at a time.

### Removed

- `/pp editor` from the help and Tab.

### Fixed

- Temporary ranks ending late.
- Removing a temporary entry also removing the permanent one.
- Permissions after a cancelled world change.
- Removed ranks coming back on join.
- Broken `config.yml` resetting settings.
- `/pp export` overwriting files.
- Unreadable database not tried again.
- Incomplete commands showing an unknown command.
- Damaged name records blocking the database.
- Group changes not reaching players through `default`.
- Two copies writing to one database.
- English text in Polish messages.

### Security

- `/pp import <file>` only from the console.
- Command blocks, signs and functions can no longer run `/pp` as the console.
- No ranks for players an import names only by nickname.
- `/tp`, `/xp`, `/banip` and `/pardonip` checked after `/execute`.
- UUIDs next to player names in replies.
- Control characters and too long texts refused.

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
