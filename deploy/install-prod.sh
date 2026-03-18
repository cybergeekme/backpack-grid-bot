#!/usr/bin/env bash
set -euo pipefail

APP_USER="backpack-grid-bot"
APP_GROUP="backpack-grid-bot"
APP_DIR="/opt/backpack-grid-bot"
ETC_DIR="/etc/backpack-grid-bot"
DATA_DIR="/var/lib/backpack-grid-bot"
SYSTEMD_UNIT="/etc/systemd/system/backpack-grid-bot.service"
WORKSPACE_DIR="/root/.openclaw/workspace"

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  echo "Please run as root: sudo bash $0"
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust first: https://rustup.rs/"
  exit 1
fi

if ! id -u "$APP_USER" >/dev/null 2>&1; then
  useradd --system --home "$APP_DIR" --shell /usr/sbin/nologin "$APP_USER"
fi

mkdir -p "$APP_DIR" "$ETC_DIR" "$DATA_DIR/runtime"
chown -R "$APP_USER:$APP_GROUP" "$APP_DIR" "$DATA_DIR"

if [[ ! -f "$WORKSPACE_DIR/Cargo.toml" || ! -d "$WORKSPACE_DIR/src" ]]; then
  echo "Workspace project not found at repo root: $WORKSPACE_DIR"
  exit 1
fi

rsync -a --delete \
  --exclude target \
  --exclude .git \
  --exclude .env \
  --exclude memory \
  --exclude skills \
  --exclude .openclaw \
  --exclude backpack-grid-bot-v1fix \
  "$WORKSPACE_DIR/" "$APP_DIR/"

cd "$APP_DIR"
cargo build --release

if [[ ! -f "$ETC_DIR/backpack-grid-bot.env" ]]; then
  cp "$APP_DIR/deploy/backpack-grid-bot.env.example" "$ETC_DIR/backpack-grid-bot.env"
  chmod 600 "$ETC_DIR/backpack-grid-bot.env"
  echo "Created env template at $ETC_DIR/backpack-grid-bot.env"
  echo "Edit it before starting the service."
fi

cp "$APP_DIR/deploy/backpack-grid-bot.service" "$SYSTEMD_UNIT"
systemctl daemon-reload
systemctl enable backpack-grid-bot.service

echo
echo "Deployment files installed."
echo "Next steps:"
echo "  1) Edit:  $ETC_DIR/backpack-grid-bot.env"
echo "  2) Start: systemctl start backpack-grid-bot.service"
echo "  3) Check: systemctl status backpack-grid-bot.service --no-pager"
echo "  4) Logs:  journalctl -u backpack-grid-bot.service -f"
