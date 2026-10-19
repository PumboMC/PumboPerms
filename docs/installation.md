# Installation

> [!TIP]
> Download the files from [Releases](https://github.com/PumboMC/PumboPerms/releases/latest), or [build from source](#building).

1. Put the file that matches your Pumpkin version into the server's `plugins/` folder, under the name from [Two builds](#two-builds).
2. Start the server. The first start creates `plugins/data/pumboperms/config.yml` and `lang/en.yml` (the file of the language set in the config).
3. From the console (or as an operator), create a group and give it permissions:

   ```
   /pp creategroup vip
   /pp group vip parent add default
   /pp group vip setweight 10
   /pp group vip permission set minecraft:command.gamemode
   /pp group vip meta setprefix 10 "&6[VIP] "
   /pp user Steve parent add vip
   ```

On PumboProx, put `pumbo-perms.wasm` into the proxy's `plugins/` folder and start the proxy. It writes `plugins/pumbo-perms/config.yml` and imports the groups and players of `permissions.yml`; the log line `permissions.yml: PumboPerms » Imported file: ...` says what it took over.

The plugin only needs to read and write its own data folder. It has no network access.

## Two builds

| Build | File in the release | Keep it in `plugins/` as | Status |
| --- | --- | --- | --- |
| **PumboProx** (whole network) | `PumboPerms-Proxy-<version>.wasm` | `pumbo-perms.wasm`, on the proxy | Stable. One set of ranks for the whole network: set them with `/pp` on the proxy and they apply on every server right away, through [PumboBridge](https://github.com/PumboMC/PumboBridge). Imports the proxy's `permissions.yml` at the first start. |
| **Pumpkin 0.2.0** (one server, Minecraft 26.3) | `PumboPerms-Pumpkin-26.3-<version>.wasm` | `PumboPerms-26.3.wasm`, on the server | Stable. Ranks and permissions for that server. |
| **Pumpkin 0.1.0-dev** (one server, Minecraft 26.2) | `PumboPerms-Pumpkin-26.2-<version>.wasm` | `PumboPerms-26.2.wasm`, on the server | Stable. Ranks and permissions for that server. |

Pumpkin checks the plugin API strictly: a file only loads on the Pumpkin version it was built for. On Pumpkin 0.1.0-dev every player sees the whole command list in Tab; commands they lack are still refused.

When PumboPerms runs on the proxy, the proxy rules. A PumboPerms on a server behind it steps back within 10 seconds: it hands the permissions it set to PumboBridge, which writes the proxy's decisions from then on, and its `/pp` answers that the proxy manages the ranks. Its own data stays untouched. The export PumboBridge receives holds only the flag that the proxy rules, whatever the size of your database and whether PumboPerms on the proxy runs right now, so a server never takes over because the proxy's PumboPerms is restarting. Remove PumboPerms from the proxy and the one on the server takes over again.

**Keeps the ranks while it is down.** When PumboPerms on the proxy restarts, is reloaded or unloaded, or cannot read its database, players online keep their permissions and the proxy refuses new logins with "Permissions are temporarily unavailable" (`on-load-failure: deny`), instead of falling back to `permissions.yml`, which still holds the ranks from before the takeover. Fix it from the proxy console with `/prox plugins reload pumbo-perms`, or set `permissions.provider: file`.

Update PumboBridge together with PumboPerms: an older PumboBridge does not list the Pumbo permissions.

## Recommended Pumpkin settings

For a server with players on it, set this in `pumpkin.toml`:

```toml
[plugins]
hot_reload = false

[plugins.overrides.pumboperms]
allowed_permissions = ["fs.read.data", "fs.write.data"]
max_memory_mb = 64
```

- `allowed_permissions` approves the only two sandbox permissions PumboPerms asks for. Write them with dots: the colon form from Pumpkin's examples never matches. Without this line every new build (a new file hash) asks in the console, hot reload does not load the plugin, and a "no" in the console stays for that file.
- With `hot_reload` on, Pumpkin reloads plugins while players play: it changes the order of event handlers, can refuse permissions, and registers commands under load.
- `max_memory_mb = 64` leaves PumboPerms room (see [Limits](how-it-works.md#limits)).
- Let only the user that runs the server write to `plugins/` and `plugins/cache/`: Pumpkin loads compiled plugin code from the cache.

## Updating

Save the new file in `plugins/` under the same name as the old one: `pumbo-perms.wasm` on PumboProx, `PumboPerms-26.3.wasm` or `PumboPerms-26.2.wasm` on Pumpkin. Keep the name, so the update overwrites the old file. On PumboProx two files with the id `pumbo-perms` stop the proxy at start with both names and "delete the older one". Your data folder stays.

Coming from 0.1.2-beta, run `/pp export` first. The first start of the new version moves the import ledgers into `perms.redb` and raises its schema; 0.1.2-beta then refuses the file as newer than it understands. To go back, put the old file in place and import your export with `/pp import <file>` on a fresh `perms.redb`.

Update PumboBridge on your servers at the same time.

## Building

You need Rust stable with the WebAssembly target:

```sh
rustup target add wasm32-wasip2
```

Cargo fetches the shared Pumbo libraries (`pumbo-common`, `pumbo-sdk`, `pumbo-contracts`) from the [PumboProx](https://github.com/PumboMC/PumboProx) repository on the first build.

Pumpkin builds (`dist/PumboPerms-26.3.wasm` and `dist/PumboPerms-26.2.wasm`):

```sh
plugins/pumbo-perms-pumpkin/build.sh
```

PumboProx build (`plugins/pumbo-perms-prox/dist/pumbo-perms.wasm`):

```sh
plugins/pumbo-perms-prox/build.sh
```

Tests of the core and both builds:

```sh
cargo test
```
