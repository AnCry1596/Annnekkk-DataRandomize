#!/usr/bin/env bash
#
# One-shot setup for annnekkk_random_server on Linux.
#
# Checks for a reachable MongoDB and installs one from the distro's packages if
# there is none, downloads the latest server release, seeds the database, and
# registers the server to launch at boot via systemd.
#
# Every step is skipped when it is already done, so re-running is safe.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/AnCry1596/Annnekkk-DataRandomize/main/setup.sh | bash
#   ./setup.sh --port 9000
#   ./setup.sh --mongo-uri 'mongodb://user:pass@10.0.0.5:27017/'
#   ./setup.sh --help

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────

INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/share/annnekkk-random-server}"
MONGO_URI="${MONGO_URI:-mongodb://localhost:27017/}"
MONGO_DB="${MONGO_DB:-random_server}"
LISTEN_HOST="${LISTEN_HOST:-127.0.0.1}"
PORT="${PORT:-8080}"
REPO="${REPO:-AnCry1596/Annnekkk-DataRandomize}"
FORCE=0
NO_AUTOSTART=0

SERVICE_NAME='annnekkk-random-server'

usage() {
    sed -n '3,15p' "$0" | sed 's/^# \{0,1\}//'
    cat <<'EOF'

Options:
  --install-dir <path>   where to install       (default: ~/.local/share/annnekkk-random-server)
  --mongo-uri <uri>      MongoDB to use         (default: mongodb://localhost:27017/)
  --mongo-db <name>      database name          (default: random_server)
  --host <addr>          listen address         (default: 127.0.0.1)
  --port <n>             listen port            (default: 8080)
  --repo <owner/name>    release source         (default: AnCry1596/Annnekkk-DataRandomize)
  --force                re-seed collections that already have documents
  --no-autostart         install but do not register the systemd service
  -h, --help             show this help
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --install-dir) INSTALL_DIR="$2"; shift 2 ;;
        --mongo-uri)   MONGO_URI="$2";   shift 2 ;;
        --mongo-db)    MONGO_DB="$2";    shift 2 ;;
        --host)        LISTEN_HOST="$2"; shift 2 ;;
        --port)        PORT="$2";        shift 2 ;;
        --repo)        REPO="$2";        shift 2 ;;
        --force)       FORCE=1;          shift ;;
        --no-autostart) NO_AUTOSTART=1;  shift ;;
        -h|--help)     usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

# ── Output helpers ────────────────────────────────────────────────────────────

if [ -t 1 ]; then
    C_STEP=$'\033[36m'; C_OK=$'\033[32m'; C_DIM=$'\033[90m'; C_WARN=$'\033[33m'; C_OFF=$'\033[0m'
else
    C_STEP=''; C_OK=''; C_DIM=''; C_WARN=''; C_OFF=''
fi

step() { printf '%s==> %s%s\n' "$C_STEP" "$1" "$C_OFF"; }
ok()   { printf '%s    %s%s\n' "$C_OK"   "$1" "$C_OFF"; }
note() { printf '%s    %s%s\n' "$C_DIM"  "$1" "$C_OFF"; }
warn() { printf '%s    %s%s\n' "$C_WARN" "$1" "$C_OFF" >&2; }
die()  { printf '%serror: %s%s\n' "$C_WARN" "$1" "$C_OFF" >&2; exit 1; }

# sudo only when not already root; absent sudo is reported rather than assumed.
if [ "$(id -u)" -eq 0 ]; then
    SUDO=''
elif command -v sudo >/dev/null 2>&1; then
    SUDO='sudo'
else
    SUDO=''
fi

need_root() {
    if [ "$(id -u)" -ne 0 ] && [ -z "$SUDO" ]; then
        die "$1 needs root, and sudo is not installed. Re-run as root."
    fi
}

# ── MongoDB helpers ───────────────────────────────────────────────────────────

# Strip scheme, credentials and path off a mongodb:// URI to get host and port.
mongo_host() {
    local hp="${1#mongodb://}"; hp="${hp#mongodb+srv://}"
    hp="${hp%%/*}"          # drop /db and query
    hp="${hp##*@}"          # drop user:pass@
    hp="${hp%%,*}"          # replica sets list several hosts; first is enough
    local h="${hp%%:*}"
    [ -n "$h" ] && printf '%s' "$h" || printf 'localhost'
}

mongo_port() {
    local hp="${1#mongodb://}"; hp="${hp#mongodb+srv://}"
    hp="${hp%%/*}"; hp="${hp##*@}"; hp="${hp%%,*}"
    case "$hp" in
        *:*) printf '%s' "${hp##*:}" ;;
        *)   printf '27017' ;;
    esac
}

# TCP liveness check with a short timeout — far quicker than a driver's
# server-selection timeout. Falls back through the tools likely to be present.
mongo_up() {
    local h p
    h="$(mongo_host "$1")"; p="$(mongo_port "$1")"
    if command -v nc >/dev/null 2>&1; then
        nc -z -w2 "$h" "$p" >/dev/null 2>&1
    elif command -v timeout >/dev/null 2>&1; then
        timeout 2 bash -c "exec 3<>/dev/tcp/$h/$p" >/dev/null 2>&1
    else
        bash -c "exec 3<>/dev/tcp/$h/$p" >/dev/null 2>&1
    fi
}

install_mongo() {
    need_root 'installing MongoDB'

    if command -v apt-get >/dev/null 2>&1; then
        note 'installing mongodb via apt-get'
        $SUDO apt-get update -qq
        # Debian/Ubuntu ship this as mongodb-server; newer releases only have
        # the mongodb package. Try both before giving up.
        $SUDO apt-get install -y -qq mongodb-server 2>/dev/null \
            || $SUDO apt-get install -y -qq mongodb \
            || die "no mongodb package in apt. Install MongoDB from https://www.mongodb.com/docs/manual/administration/install-on-linux/ and re-run."
    elif command -v dnf >/dev/null 2>&1; then
        note 'installing mongodb via dnf'
        $SUDO dnf install -y mongodb-server \
            || die "no mongodb-server package in dnf. Install MongoDB from https://www.mongodb.com/docs/manual/administration/install-on-linux/ and re-run."
    elif command -v pacman >/dev/null 2>&1; then
        note 'installing mongodb via pacman'
        # Arch moved mongodb to the AUR; mongodb-bin is the usual stand-in.
        $SUDO pacman -Sy --noconfirm mongodb-bin 2>/dev/null \
            || die "mongodb is in the AUR on Arch. Install mongodb-bin with an AUR helper and re-run."
    elif command -v zypper >/dev/null 2>&1; then
        note 'installing mongodb via zypper'
        $SUDO zypper --non-interactive install mongodb \
            || die "no mongodb package in zypper. Install MongoDB manually and re-run."
    else
        die "no supported package manager found (apt-get, dnf, pacman, zypper). Install MongoDB manually and re-run."
    fi

    # Package names for the service differ across distros.
    if command -v systemctl >/dev/null 2>&1; then
        for svc in mongod mongodb; do
            if systemctl list-unit-files 2>/dev/null | grep -q "^${svc}\.service"; then
                $SUDO systemctl enable --now "$svc" >/dev/null 2>&1 || true
                break
            fi
        done
    fi
}

# ── 1. MongoDB ────────────────────────────────────────────────────────────────

step 'Checking MongoDB'
MHOST="$(mongo_host "$MONGO_URI")"
MPORT="$(mongo_port "$MONGO_URI")"

if mongo_up "$MONGO_URI"; then
    ok "reachable at $MHOST:$MPORT"
else
    case "$MHOST" in
        localhost|127.0.0.1|::1)
            note "nothing listening on $MHOST:$MPORT - installing MongoDB"
            install_mongo

            note 'waiting for MongoDB to accept connections'
            deadline=$(( $(date +%s) + 90 ))
            until mongo_up "$MONGO_URI" || [ "$(date +%s)" -ge "$deadline" ]; do
                sleep 2
            done
            mongo_up "$MONGO_URI" \
                || die 'MongoDB was installed but never started listening. Check: systemctl status mongod'
            ok "installed and listening on $MHOST:$MPORT"
            ;;
        *)
            # A remote host we cannot reach is a config problem; installing a
            # local server would not hold the expected data.
            die "cannot reach MongoDB at $MHOST:$MPORT. Check the host, port and firewall, or pass --mongo-uri for a different server."
            ;;
    esac
fi

# ── 2. Server binaries ────────────────────────────────────────────────────────

step 'Installing server'
mkdir -p "$INSTALL_DIR"

case "$(uname -m)" in
    x86_64|amd64)  ASSET='x86_64-unknown-linux-gnu.tar.gz' ;;
    aarch64|arm64) ASSET='aarch64-unknown-linux-gnu.tar.gz' ;;
    *) die "unsupported architecture: $(uname -m) (releases cover x86_64 and aarch64)" ;;
esac

command -v curl >/dev/null 2>&1 || die 'curl is required'
command -v tar  >/dev/null 2>&1 || die 'tar is required'

note "fetching latest release of $REPO"
API="https://api.github.com/repos/$REPO/releases/latest"
RELEASE_JSON="$(curl -fsSL -H 'Accept: application/vnd.github+json' -H 'User-Agent: annnekkk-setup' "$API")" \
    || die "cannot query $API"

# Pull the asset URL without depending on jq being installed.
URL="$(printf '%s' "$RELEASE_JSON" \
    | tr ',' '\n' \
    | grep '"browser_download_url"' \
    | grep -F "$ASSET" \
    | head -n1 \
    | sed 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"
[ -n "$URL" ] || die "the latest release of $REPO has no $ASSET"

TAG="$(printf '%s' "$RELEASE_JSON" | tr ',' '\n' | grep '"tag_name"' | head -n1 \
    | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"

# A running instance would keep the old inode; stop it before replacing.
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user stop "$SERVICE_NAME" >/dev/null 2>&1 || true
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/release.tar.gz" || die "downloading $URL failed"
tar xzf "$TMP/release.tar.gz" -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR"/annnekkk_random_server "$INSTALL_DIR"/seed "$INSTALL_DIR"/dump 2>/dev/null || true
ok "${TAG:-latest} -> $INSTALL_DIR"

EXE="$INSTALL_DIR/annnekkk_random_server"
[ -x "$EXE" ] || die 'archive did not contain annnekkk_random_server'

# ── 3. Configuration ──────────────────────────────────────────────────────────

step 'Writing .env'
ENV_PATH="$INSTALL_DIR/.env"
# The URI can hold a password, so keep the file owner-readable only.
umask 077
cat > "$ENV_PATH" <<EOF
MONGODB_URI=$MONGO_URI
MONGODB_DB=$MONGO_DB

RUST_LOG=info

HOST=$LISTEN_HOST
PORT=$PORT
EOF
umask 022
ok "$ENV_PATH"

# ── 4. Seed ───────────────────────────────────────────────────────────────────

step 'Seeding database'
if [ -x "$INSTALL_DIR/seed" ]; then
    # seed downloads data.zip itself when the data dir is empty, and skips
    # collections that already have documents unless --force is passed.
    seed_args=("$MONGO_URI" "$MONGO_DB")
    [ "$FORCE" -eq 1 ] && seed_args+=(--force)
    ( cd "$INSTALL_DIR" && ./seed "${seed_args[@]}" ) || die 'seed failed'
    ok 'seeded'
else
    note 'seed binary not in the archive - skipping'
fi

# ── 5. Launch at boot ─────────────────────────────────────────────────────────

if [ "$NO_AUTOSTART" -eq 1 ]; then
    step 'Skipping service registration (--no-autostart)'
elif ! command -v systemctl >/dev/null 2>&1; then
    step 'Skipping service registration'
    note 'systemd not found - start manually with:'
    note "  cd $INSTALL_DIR && ./annnekkk_random_server"
else
    step 'Registering systemd service'
    # A user unit rather than a system one: it needs no root, and the binary
    # only serves local traffic by default.
    UNIT_DIR="$HOME/.config/systemd/user"
    mkdir -p "$UNIT_DIR"
    cat > "$UNIT_DIR/$SERVICE_NAME.service" <<EOF
[Unit]
Description=annnekkk random data server
After=network-online.target

[Service]
Type=simple
WorkingDirectory=$INSTALL_DIR
ExecStart=$EXE
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable "$SERVICE_NAME" >/dev/null 2>&1 || true
    systemctl --user restart "$SERVICE_NAME"
    ok "user service '$SERVICE_NAME' (starts at login)"

    # User services stop at logout unless lingering is on. Best-effort: this
    # needs root, and the service still works without it while logged in.
    if command -v loginctl >/dev/null 2>&1 && { [ -n "$SUDO" ] || [ "$(id -u)" -eq 0 ]; }; then
        if $SUDO loginctl enable-linger "$USER" >/dev/null 2>&1; then
            note 'lingering enabled - service also runs while logged out'
        fi
    fi

    # Confirm it actually serves traffic rather than just reporting success.
    # Startup loads the whole reference-data cache before binding, which against
    # a remote MongoDB can take a couple of minutes on a cold connection.
    WAIT=180
    note "waiting for the server to respond (up to ${WAIT}s)"
    URL_CHECK="http://$LISTEN_HOST:$PORT/randomdatav2/new"
    deadline=$(( $(date +%s) + WAIT ))
    served=0
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if curl -fsS -o /dev/null --max-time 5 "$URL_CHECK" 2>/dev/null; then
            served=1; break
        fi
        sleep 2
    done
    if [ "$served" -eq 1 ]; then
        ok "serving on http://$LISTEN_HOST:$PORT"
    else
        warn "no response on $URL_CHECK after ${WAIT}s. It may still be loading - check with:"
        warn "  systemctl --user status $SERVICE_NAME"
        warn "  journalctl --user -u $SERVICE_NAME -n 50"
    fi
fi

printf '\n%sDone.%s\n' "$C_OK" "$C_OFF"
printf '%s  server    http://%s:%s%s\n' "$C_DIM" "$LISTEN_HOST" "$PORT" "$C_OFF"
printf '%s  try       curl "http://%s:%s/randomdatav2/new"%s\n' "$C_DIM" "$LISTEN_HOST" "$PORT" "$C_OFF"
printf '%s  config    %s%s\n' "$C_DIM" "$ENV_PATH" "$C_OFF"
if [ "$NO_AUTOSTART" -eq 0 ] && command -v systemctl >/dev/null 2>&1; then
    printf '%s  logs      journalctl --user -u %s -f%s\n' "$C_DIM" "$SERVICE_NAME" "$C_OFF"
    printf '%s  uninstall systemctl --user disable --now %s%s\n' "$C_DIM" "$SERVICE_NAME" "$C_OFF"
fi
