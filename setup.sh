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
# -n so a missing password fails immediately: piped into bash there is no TTY,
# and an interactive prompt would hang the script indefinitely.
if [ "$(id -u)" -eq 0 ]; then
    SUDO=''
elif command -v sudo >/dev/null 2>&1; then
    SUDO='sudo -n'
else
    SUDO=''
fi

need_root() {
    [ "$(id -u)" -eq 0 ] && return 0
    if [ -z "$SUDO" ]; then
        die "$1 needs root, and sudo is not installed. Re-run as root."
    fi
    # Piped into bash there is no TTY, so a sudo that wants a password would
    # hang forever. Check up front and say so instead.
    if ! sudo -n true 2>/dev/null; then
        die "$1 needs root. sudo requires a password here, and this script cannot prompt.
  Run one of these instead:
    sudo -v && curl -fsSL <url> | bash     # cache credentials first
    curl -fsSL <url> -o setup.sh && sudo bash setup.sh"
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

MONGO_MAJOR="${MONGO_MAJOR:-8.0}"

# Add MongoDB's official apt repo. Ubuntu 22.04+ and Debian 12+ dropped the
# distro-packaged mongodb entirely, so this is the only apt route on any
# current release.
add_mongo_apt_repo() {
    local id codename suite url
    # shellcheck disable=SC1091
    . /etc/os-release
    id="${ID:-}"
    # VERSION_CODENAME is absent on some derivatives; fall back to the parent.
    codename="${VERSION_CODENAME:-}"
    [ -z "$codename" ] && codename="${UBUNTU_CODENAME:-}"

    case "$id" in
        ubuntu|linuxmint|pop|elementary|zorin) url='https://repo.mongodb.org/apt/ubuntu'; suite="$codename" ;;
        debian|raspbian)                       url='https://repo.mongodb.org/apt/debian'; suite="$codename" ;;
        *)  # Derivatives set ID_LIKE; use it to pick the right pool.
            case "${ID_LIKE:-}" in
                *ubuntu*) url='https://repo.mongodb.org/apt/ubuntu'; suite="$codename" ;;
                *debian*) url='https://repo.mongodb.org/apt/debian'; suite="$codename" ;;
                *) return 1 ;;
            esac ;;
    esac

    # Only suites MongoDB actually publishes. An unknown codename (a very new
    # or very old release) maps to the closest supported one.
    case "$suite" in
        focal|jammy|noble|bookworm|trixie) ;;
        # Ubuntu derivatives commonly track these bases.
        una|vanessa|vera|victoria|virginia) suite='jammy' ;;
        wilma|xia|zara)                     suite='noble' ;;
        *)
            case "$url" in
                *ubuntu) suite='noble' ;;
                *)       suite='bookworm' ;;
            esac
            note "unrecognised codename '${codename:-none}' - using $suite packages" ;;
    esac

    note "adding MongoDB $MONGO_MAJOR repo ($suite)"
    $SUDO apt-get install -y -qq curl gnupg ca-certificates >/dev/null 2>&1 || true
    $SUDO install -d -m 0755 /usr/share/keyrings

    # gpg --dearmor writes a binary keyring; signed-by pins the repo to it.
    curl -fsSL "https://pgp.mongodb.com/server-${MONGO_MAJOR}.asc" \
        | $SUDO gpg --dearmor --yes -o "/usr/share/keyrings/mongodb-server-${MONGO_MAJOR}.gpg" \
        || return 1

    local component='main'
    case "$url" in *debian) component='main' ;; *) component='multiverse' ;; esac

    printf 'deb [ arch=amd64,arm64 signed-by=/usr/share/keyrings/mongodb-server-%s.gpg ] %s %s/mongodb-org/%s %s\n' \
        "$MONGO_MAJOR" "$url" "$suite" "$MONGO_MAJOR" "$component" \
        | $SUDO tee "/etc/apt/sources.list.d/mongodb-org-${MONGO_MAJOR}.list" >/dev/null || return 1

    $SUDO apt-get update -qq
}

install_mongo() {
    need_root 'installing MongoDB'

    if command -v apt-get >/dev/null 2>&1; then
        note 'installing mongodb via apt-get'
        $SUDO apt-get update -qq || true
        # Try the distro package first — present on older releases and already
        # mirrored locally. Current releases have neither, so fall through to
        # MongoDB's own repo.
        if $SUDO apt-get install -y -qq mongodb-server >/dev/null 2>&1 \
            || $SUDO apt-get install -y -qq mongodb >/dev/null 2>&1; then
            :
        else
            add_mongo_apt_repo \
                || die "cannot add the MongoDB apt repo. Install it manually: https://www.mongodb.com/docs/manual/administration/install-on-linux/"
            $SUDO apt-get install -y -qq mongodb-org \
                || die "installing mongodb-org failed. See https://www.mongodb.com/docs/manual/administration/install-on-linux/"
        fi
    elif command -v dnf >/dev/null 2>&1; then
        note 'installing mongodb via dnf'
        # Fedora/RHEL dropped mongodb-server too; fall back to MongoDB's repo.
        if ! $SUDO dnf install -y mongodb-server >/dev/null 2>&1; then
            local rel
            # shellcheck disable=SC1091
            . /etc/os-release
            case "${ID:-}" in
                fedora) rel=9 ;;   # no Fedora pool; the el9 packages work
                *)      rel="$(printf '%s' "${VERSION_ID:-9}" | cut -d. -f1)" ;;
            esac
            note "adding MongoDB $MONGO_MAJOR repo (el$rel)"
            printf '[mongodb-org-%s]\nname=MongoDB\nbaseurl=https://repo.mongodb.org/yum/redhat/%s/mongodb-org/%s/x86_64/\ngpgcheck=1\nenabled=1\ngpgkey=https://pgp.mongodb.com/server-%s.asc\n' \
                "$MONGO_MAJOR" "$rel" "$MONGO_MAJOR" "$MONGO_MAJOR" \
                | $SUDO tee "/etc/yum.repos.d/mongodb-org-${MONGO_MAJOR}.repo" >/dev/null \
                || die "cannot add the MongoDB yum repo. Install it manually: https://www.mongodb.com/docs/manual/administration/install-on-linux/"
            $SUDO dnf install -y mongodb-org \
                || die "installing mongodb-org failed. See https://www.mongodb.com/docs/manual/administration/install-on-linux/"
        fi
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

    # Package names for the service differ across distros: the distro packages
    # use mongodb, MongoDB's own use mongod.
    if command -v systemctl >/dev/null 2>&1; then
        local started='' svc attempt
        # The unit file lands during dpkg configuration and is not visible to
        # systemd until it reloads, so look on disk (authoritative the moment
        # dpkg writes it) and retry briefly rather than trusting one lookup.
        for attempt in 1 2 3 4 5; do
            $SUDO systemctl daemon-reload >/dev/null 2>&1 || true
            for svc in mongod mongodb; do
                if [ -f "/usr/lib/systemd/system/${svc}.service" ] \
                    || [ -f "/lib/systemd/system/${svc}.service" ] \
                    || [ -f "/etc/systemd/system/${svc}.service" ]; then
                    started="$svc"
                    break
                fi
            done
            [ -n "$started" ] && break
            sleep 2
        done

        if [ -n "$started" ]; then
            # Report failure rather than swallowing it — a service that never
            # starts is the difference between working and not.
            $SUDO systemctl enable --now "$started" >/dev/null 2>&1 \
                || warn "systemctl enable --now $started failed"
        else
            warn 'no mongod/mongodb service unit found to start'
        fi
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
            mongo_up "$MONGO_URI" || die "MongoDB was installed but never started listening. Check:
    systemctl status mongod
    journalctl -u mongod -n 50
    tail /var/log/mongodb/mongod.log"
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
    # Best effort: $SUDO carries -n, so a password prompt fails fast rather than
    # hanging, and the service still works while logged in either way.
    if command -v loginctl >/dev/null 2>&1; then
        if $SUDO loginctl enable-linger "$USER" >/dev/null 2>&1; then
            note 'lingering enabled - service also runs while logged out'
        else
            note 'service runs while logged in; for logged-out operation run:'
            note "  sudo loginctl enable-linger $USER"
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
