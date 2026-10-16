#!/usr/bin/env bash
# The live scenario of PumboPerms: PumboProx from the checkout in PUMBO_PROX_DIR
# with two Pumpkin servers behind it (ports 25700-25703,
# through tools/dev-network.sh of PumboProx), then a lone Pumpkin on 25704. The
# bots of pumbo-testclient's `perms_live` example play the steps and read the
# logs; this script writes the data, builds, starts, restarts and kills.
#
# Usage: tools/perms-live.sh [--small]
#
#   --small        permissions.yml without the 3000 players of vip
#
# Variables: NETWORK_STEPS (N1 N2 N3 N4 N5 N5b N6 N7 N8 N9 N15 N12; N13 is the hour-long
# run, only when named; N14 needs a backend that keeps a 35 s configuration phase
# open, Pumpkin 0.2.0 does not), PERMS_LIVE_N12_SECS (300) and PERMS_LIVE_N13_SECS
# (3600) shorten the load and the long run, RESTART (1: then
# N10 after down and up, N11 after a SIGKILL of the proxy and survival),
# SOLO_STEPS (J1 J2 J3 J7 J4 J5 J6 J8; J7 before J4, which loads PumboPerms
# again); an empty list skips its phase. J8 starts the
# released PumboPerms 0.1.2-beta (OLD_PERMS_WASM), makes data, swaps in this
# build, checks the data and goes back to the old file, which has to refuse the
# newer database and leave its data as it is (this build reads it again). PUMPKIN_BIN
# (the Pumpkin binary), PUMBO_PROTOCOL of that Pumpkin (777 for
# 0.2.0+26.3-26.51; PumboPerms is built for Minecraft 26.3 only for now),
# PUMBO_PROX_DIR (a PumboProx checkout), PUMBO_BRIDGE_DIR and PUMBO_BANS_DIR (checkouts
# that build PumboBridge and PumboBans; this repository when it has them).
# LAYOUT runs one setup instead of both phases:
#   proxy     PumboPerms on the proxy only, the servers without PumboBridge and
#             PumboPerms; steps that need the bridge print a SKIP line, and N1,
#             N10 and N11 run their proxy parts (N1p, N10p, N11p)
#   pumpkin   PumboPerms on a lone Pumpkin (the J phase)
#   bridge    PumboPerms on the proxy, PumboBridge and a passive PumboPerms on
#             the servers (the N phase)
# Every phase starts from a fresh install with default configs; only ports and
# paths are set. Processes are stopped by the PIDs this script started.
set -euo pipefail
shopt -s nullglob

L=$(cd "$(dirname "$0")/.." && pwd)
P=$(cd "${PUMBO_PROX_DIR:?a PumboProx checkout}" && pwd)
# PERMS_LIVE_RESULTS: a folder that gets the log and a summary of each run
# (its parent folder has to exist).
RESULTS=${PERMS_LIVE_RESULTS-}
RUN_LOG=$(mktemp "${TMPDIR:-/tmp}/perms-live.XXXXXX")
exec > >(tee "$RUN_LOG") 2>&1
PUMPKIN=${PUMPKIN_BIN:?a Pumpkin binary}
NETWORK_STEPS=${NETWORK_STEPS-N1 N2 N3 N4 N5 N5b N6 N7 N8 N9 N15 N12}
SOLO_STEPS=${SOLO_STEPS-J1 J2 J3 J7 J4 J5 J6 J8}
RESTART=${RESTART-1}
SMALL=0
[ "${1:-}" = --small ] && SMALL=1
PUMBO_PROTOCOL=${PUMBO_PROTOCOL:-777}
case $PUMBO_PROTOCOL in
  777) VARIANT=26.3 ;;
  *) echo "perms-live: PumboPerms is built for Minecraft 26.3 only for now (PUMBO_PROTOCOL $PUMBO_PROTOCOL is not tested)" >&2; exit 1 ;;
esac
OLD_PERMS_WASM=${OLD_PERMS_WASM:-}
BRIDGE_DIR=${PUMBO_BRIDGE_DIR:-$L}
BANS_DIR=${PUMBO_BANS_DIR:-$L}
# Without the bridge, N1, N10 and N11 run only their proxy parts.
half() { echo "SKIP $1: the server part needs PumboBridge, ${1}p checks the proxy"; }
P_PART=
case ${LAYOUT:-} in
  '' | bridge) ;;
  proxy)
    SERVER_PLUGINS=PumboBans
    P_PART=p
    steps=
    for s in $NETWORK_STEPS; do
      case $s in
        N1) half N1; steps+=" N1p" ;;
        # They read what PumboBridge writes on the servers.
        N3 | N4 | N6 | N14 | N15) echo "SKIP $s: needs PumboBridge on the servers" ;;
        *) steps+=" $s" ;;
      esac
    done
    NETWORK_STEPS=${steps# }
    ;;
  pumpkin) NETWORK_STEPS= ;;
  *) echo "perms-live: LAYOUT $LAYOUT: proxy, pumpkin or bridge" >&2; exit 1 ;;
esac
case ${LAYOUT:-} in proxy | bridge) SOLO_STEPS= ;; esac

PORT=25700
SOLO_PORT=25704
NET=$P/target/perms-live
SOLO=$NET/solo
UPGRADE=$NET/upgrade
PLUGINS=$L/target/live-plugins
export PATH="$HOME/.cargo/bin:$PATH"
export PUMBO_PORT=$PORT PUMBO_PROTOCOL PUMBO_PLUGINS_DIR=$PLUGINS PUMBO_NETWORK_DIR=$NET

die() { echo "perms-live: $*" >&2; exit 1; }
network() { "$P/tools/dev-network.sh" "$@"; }
port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }
# Whatever listens on the test ports: lsof, else ss, else /proc (Linux).
listeners() {
  if command -v lsof >/dev/null; then
    lsof -nP -iTCP:$PORT-$SOLO_PORT -sTCP:LISTEN 2>/dev/null || true
  elif command -v ss >/dev/null; then
    ss -Hltnp "sport >= :$PORT and sport <= :$SOLO_PORT"
  else
    local _ addr state p
    while read -r _ addr _ state _; do
      p=$((16#${addr##*:}))
      [ "$state" = 0A ] && [ "$p" -ge $PORT ] && [ "$p" -le $SOLO_PORT ] && echo "port $p (/proc/net/tcp)"
    done < <(tail -q -n +2 /proc/net/tcp /proc/net/tcp6 2>/dev/null)
  fi
  true
}

# One test run at a time on this machine; the lock goes away on any exit.
if [ "${PERMS_TEST_LOCK_HELD:-}" != 1 ]; then
  until mkdir /tmp/perms-live.lock 2>/dev/null; do sleep 5; done
  LOCKED=1
fi
solo_stop() { # <dir>
  [ -f "$1/pid" ] || return 0
  kill -TERM "$(cat "$1/pid")" 2>/dev/null || true
  for _ in $(seq 1 60); do kill -0 "$(cat "$1/pid")" 2>/dev/null || break; sleep 0.5; done
  kill -KILL "$(cat "$1/pid")" 2>/dev/null || true
  rm -f "$1/pid"
}
cleanup() {
  [ -d "$NET" ] && network down || true
  solo_stop "$SOLO"
  solo_stop "$UPGRADE"
  # An if, not &&: under set -e a false test here would end on_exit before record.
  if [ "${LOCKED:-}" = 1 ]; then rmdir /tmp/perms-live.lock; fi
}
record() { # <exit code>
  [ -d "${RESULTS%/*}" ] || { rm -f "$RUN_LOG"; return 0; }
  mkdir -p "$RESULTS/logi"
  sleep 0.5 # the last lines through tee
  local stamp
  stamp=$(date +%Y%m%d-%H%M)
  cp "$RUN_LOG" "$RESULTS/logi/$stamp-perms-live.log"
  {
    echo
    echo "## $(date '+%Y-%m-%d %H:%M') $([ "$1" = 0 ] && echo OK || echo "FAIL (exit $1)")"
    echo "- code: $(basename "$L") $(git -C "$L" rev-parse --short HEAD)$(changed "$L"), $(basename "$P") $(git -C "$P" rev-parse --short HEAD)$(changed "$P")"
    echo "- steps: $(grep -oE '^(PASS|FAIL|SKIP) [A-Za-z0-9]+' "$RUN_LOG" | tr '\n' ' ' || true)"
    echo "- retries: $(grep -c '^RETRY' "$RUN_LOG" || true)"
    grep -E '^[NJ][0-9]+[a-z]?: .*[0-9]+ ms' "$RUN_LOG" | sed 's/^/- /' || true
    grep -E '^perms-live: ' "$RUN_LOG" | tail -1 | sed 's/^/- /' || true
    echo "- log: logi/$stamp-perms-live.log"
  } >> "$RESULTS/perms-live.md"
  rm -f "$RUN_LOG"
}
changed() { [ -z "$(git -C "$1" status --porcelain)" ] || echo " + uncommitted"; }
on_exit() { cleanup; record "$1"; }
trap 'on_exit $?' EXIT

[ -x "$PUMPKIN" ] || die "no Pumpkin binary at $PUMPKIN (PUMPKIN_BIN)"
for d in "$BRIDGE_DIR/plugins/pumbo-bridge-pumpkin" "$BANS_DIR/plugins/pumbo-bans-pumpkin"; do
  [ -x "$d/build.sh" ] || die "no $d/build.sh (PUMBO_BRIDGE_DIR, PUMBO_BANS_DIR)"
done
case " $SOLO_STEPS " in *" J8 "*) [ -f "$OLD_PERMS_WASM" ] || die "no PumboPerms 0.1.2-beta at $OLD_PERMS_WASM (OLD_PERMS_WASM)" ;; esac
busy=$(listeners)
if [ -n "$busy" ]; then
  echo "$busy" >&2
  die "ports $PORT-$SOLO_PORT are in use"
fi

echo "== build"
"$L/plugins/pumbo-perms-prox/build.sh"
"$L/plugins/pumbo-perms-pumpkin/build.sh"
"$BRIDGE_DIR/plugins/pumbo-bridge-pumpkin/build.sh"
"$BANS_DIR/plugins/pumbo-bans-pumpkin/build.sh"
rm -rf "$PLUGINS"
mkdir -p "$PLUGINS"
cp "$L/plugins/pumbo-perms-prox/dist/pumbo-perms.wasm" "$L/dist/PumboPerms-$VARIANT.wasm" \
  "$BRIDGE_DIR/dist/PumboBridge-$VARIANT.wasm" "$BANS_DIR/plugins/pumbo-bans-pumpkin/dist/PumboBans-$VARIANT.wasm" "$PLUGINS/"
# SERVER_PLUGINS picks which Pumbo plugins the network's servers get (default:
# all three; LAYOUT=proxy: PumboBans), to tell which one a measurement belongs to.
for p in PumboBridge PumboPerms PumboBans; do
  case " ${SERVER_PLUGINS-PumboBridge PumboPerms PumboBans} " in
    *" $p "*) ;;
    *) mv "$PLUGINS/$p-$VARIANT.wasm" "$PLUGINS/.$p-$VARIANT.wasm.off" ;;
  esac
done
(cd "$P" && cargo build -p pumbo-testclient --example perms_live)
DRIVER=${CARGO_TARGET_DIR:-$P/target}/debug/examples/perms_live
uuid() { "$DRIVER" --uuid "$1"; }
solo_uuid() { "$DRIVER" --solo-uuid "$1"; }

ops() { # <file> (<nick> <uuid> <level>)...
  local file=$1 sep=
  shift
  {
    echo "["
    while [ $# -gt 0 ]; do
      printf '%s  {"uuid": "%s", "name": "%s", "level": %s, "bypasses_player_limit": false}\n' "$sep" "$2" "$1" "$3"
      sep=,
      shift 3
    done
    echo "]"
  } > "$file"
}

lock_moves() { # <data folder>: with nobody playing, the periodic task alone keeps the lock fresh
  local before
  before=$(cut -d' ' -f2 "$1/perms.redb.lock") || die "no perms.redb.lock in $1"
  sleep 40
  [ "$(cut -d' ' -f2 "$1/perms.redb.lock")" != "$before" ] || die "perms.redb.lock in $1 was not refreshed in 40 idle seconds"
  echo "PASS idle lock refresh ($1)"
}

# A killed Pumpkin leaves a fresh lock: the restart refuses the database, then
# opens it once the lock is 30 s old. A restart later than that opens it at once.
lock_after_kill() { # <console.log> <lines before the restart> <seconds from the kill to the restart> <label>
  local new opened=
  for _ in $(seq 1 120); do
    new=$(tail -n +"$(($2 + 1))" "$1")
    if grep -q "PumboPerms: the database is open now" <<<"$new"; then opened=refused; break; fi
    if grep -q "PumboPerms: database opened in" <<<"$new" && ! grep -q "open in another instance" <<<"$new"; then
      opened=direct
      break
    fi
    sleep 0.5
  done
  case $opened in
    refused) echo "PASS $4 lock: refused the lock left by the kill, opened when it went stale" ;;
    direct)
      [ "$3" -ge 30 ] || die "$4: the restart $3 s after the kill opened the database despite the fresh lock"
      echo "PASS $4 lock: the restart came $3 s after the kill, the lock was stale" ;;
    *) die "$4: PumboPerms did not open its database after the kill, see $1" ;;
  esac
}

run_network() { # <step>...
  echo "== network: $*"
  (cd "$NET" && "$DRIVER" network "127.0.0.1:$PORT" "$NET" "$@")
}

if [ -n "$NETWORK_STEPS" ]; then
  echo "== network: fresh install in $NET"
  # The directory is this script's own; dev-network.sh deletes only what it created.
  if [ -f "$NET/.dev-network" ]; then network reset; else rm -rf "$NET"; fi
  mkdir -p "$NET/proxy" "$NET/survival/data"
  {
    echo "# Written by tools/perms-live.sh."
    echo "groups:"
    echo "  helper: {}"
    echo "  builder: {}"
    echo "  mod: {}"
    echo "  vip: { permissions: [\"minecraft:command.gamemode\"] }"
    echo "players:"
    echo "  $(uuid PpAdm7x): { permissions: [\"*\"] }"
    echo "  $(uuid PpMod7x): { groups: [mod] }"
    echo "  $(uuid PpSlow7x): { groups: [mod] }"
    echo "  PpPend7x: { groups: [mod] }"
    if [ "$SMALL" = 0 ]; then
      for i in $(seq 1 3000); do printf '  00000000-0000-4000-8000-%012d: { groups: [vip] }\n' "$i"; done
    fi
  } > "$NET/proxy/permissions.yml"
  # The load bots of N12 and N13 are ops: Pumpkin kicks anyone else who sends more than about a command a second.
  ops "$NET/survival/data/ops.json" PpOp7x "$(uuid PpOp7x)" 3 PpPlain7x "$(uuid PpPlain7x)" 1 \
    PpPend7x "$(uuid PpPend7x)" 1 PpSlow7x "$(uuid PpSlow7x)" 1
  network up "$PUMPKIN"
  # dev-network.sh keeps files it did not write.
  grep -q PpPend7x "$NET/proxy/permissions.yml" || die "permissions.yml was overwritten"
  grep -q PpOp7x "$NET/survival/data/ops.json" || die "ops.json was overwritten"
  # shellcheck disable=SC2086
  run_network $NETWORK_STEPS
  if [ "$RESTART" = 1 ]; then
    network down
    network up "$PUMPKIN"
    [ -z "$P_PART" ] || half N10
    run_network N10$P_PART
    # Without the bridge the servers have no PumboPerms and no lock to check.
    [ -n "$P_PART" ] || lock_moves "$NET/survival/plugins/data/pumboperms"
    kill -KILL "$(cut -d' ' -f1 "$NET/proxy/pid")" "$(cut -d' ' -f1 "$NET/survival/pid")"
    killed=$(date +%s)
    sleep 1
    # N11 reads the time of opening the database from a fresh log.
    mv "$NET/proxy/console.log" "$NET/proxy/console.before-kill.log"
    lines=$(wc -l < "$NET/survival/console.log")
    # `up` builds the proxy only after a change, so survival may start while its lock is still fresh.
    network up "$PUMPKIN"
    [ -n "$P_PART" ] || lock_after_kill "$NET/survival/console.log" "$lines" "$(($(date +%s) - killed))" "N11 survival"
    [ -z "$P_PART" ] || half N11
    run_network N11$P_PART
  fi
  network down
fi

solo_setup() { # <dir> <PumboPerms file>: a fresh lone Pumpkin with that PumboPerms
  rm -rf "$1"
  mkdir -p "$1/plugins" "$1/data"
  cp "$2" "$1/plugins/PumboPerms-$VARIANT.wasm"
  cp "$PLUGINS/PumboBans-$VARIANT.wasm" "$1/plugins/"
  ops "$1/data/ops.json" PpAdm7x "$(solo_uuid PpAdm7x)" 4 PpOp7x "$(solo_uuid PpOp7x)" 3
  cat > "$1/pumpkin.toml" <<EOF
# Written by tools/perms-live.sh. Pumpkin fills in the options missing here.
[networking.java]
address = "127.0.0.1:$SOLO_PORT"
online_mode = false
encryption = false
motd = "solo"

[networking.bedrock]
enabled = false

[networking.lan_broadcast]
enabled = false

[commands]
use_tty = false

[plugins]
ask_permission_confirmation = false

[telemetry]
enabled = false
EOF
}

solo_start() { # <dir>
  (
    cd "$1"
    nohup "$PUMPKIN" < /dev/null >> console.log 2>&1 &
    echo $! > pid
  )
  for _ in $(seq 1 240); do
    port_open $SOLO_PORT && return 0
    kill -0 "$(cat "$1/pid")" 2>/dev/null || die "the lone Pumpkin stopped, see $1/console.log"
    sleep 0.5
  done
  die "the lone Pumpkin does not listen on $SOLO_PORT, see $1/console.log"
}

run_solo() { # <dir> <step>...
  local dir=$1
  shift
  echo "== solo: $*"
  (cd "$dir" && "$DRIVER" solo "127.0.0.1:$SOLO_PORT" "$dir" "$@")
}

SOLO_MAIN=${SOLO_STEPS//J8/}
if [ -n "${SOLO_MAIN// /}" ]; then
  echo "== solo: fresh install in $SOLO"
  solo_setup "$SOLO" "$PLUGINS/PumboPerms-$VARIANT.wasm"
  # `/plugin load` of Pumpkin 0.2.0 reads from the working directory (J4).
  cp "$PLUGINS/PumboPerms-$VARIANT.wasm" "$SOLO/"
  solo_start "$SOLO"
  # shellcheck disable=SC2086
  run_solo "$SOLO" $SOLO_MAIN
  lock_moves "$SOLO/plugins/data/pumboperms"
  # A crash and a restart 1 s later, as systemd's Restart=always does.
  kill -KILL "$(cat "$SOLO/pid")"
  killed=$(date +%s)
  rm -f "$SOLO/pid"
  lines=$(wc -l < "$SOLO/console.log")
  sleep 1
  solo_start "$SOLO"
  lock_after_kill "$SOLO/console.log" "$lines" "$(($(date +%s) - killed))" "solo"
  solo_stop "$SOLO"
fi

case " $SOLO_STEPS " in *" J8 "*)
  echo "== solo: J8, PumboPerms 0.1.2-beta in $UPGRADE"
  DB=$UPGRADE/plugins/data/pumboperms/perms.redb
  solo_setup "$UPGRADE" "$OLD_PERMS_WASM"
  solo_start "$UPGRADE"
  run_solo "$UPGRADE" J8a
  solo_stop "$UPGRADE"
  cp "$DB" "$UPGRADE/perms-0.1.2-beta.redb"
  echo "== solo: J8, this build on the same data"
  cp "$PLUGINS/PumboPerms-$VARIANT.wasm" "$UPGRADE/plugins/PumboPerms-$VARIANT.wasm"
  solo_start "$UPGRADE"
  run_solo "$UPGRADE" J8b
  solo_stop "$UPGRADE"
  echo "== solo: J8, back to 0.1.2-beta"
  lines=$(wc -l < "$UPGRADE/console.log")
  cp "$OLD_PERMS_WASM" "$UPGRADE/plugins/PumboPerms-$VARIANT.wasm"
  solo_start "$UPGRADE"
  refused=
  for _ in $(seq 1 120); do
    if tail -n +"$((lines + 1))" "$UPGRADE/console.log" | grep -q "newer than this version understands"; then
      refused=1
      break
    fi
    sleep 0.5
  done
  solo_stop "$UPGRADE"
  [ -n "$refused" ] || die "J8: 0.1.2-beta did not refuse the newer database, see $UPGRADE/console.log"
  # redb rewrites its header when it closes a file, so the data decides: this build reads it all again.
  echo "== solo: J8, this build again after 0.1.2-beta"
  cp "$PLUGINS/PumboPerms-$VARIANT.wasm" "$UPGRADE/plugins/PumboPerms-$VARIANT.wasm"
  solo_start "$UPGRADE"
  run_solo "$UPGRADE" J8b
  solo_stop "$UPGRADE"
  echo "PASS J8"
  ;;
esac

echo
echo "perms-live: OK"
