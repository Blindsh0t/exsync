#!/usr/bin/env bash
set -euo pipefail

# Install exsync: build the release binary, install it with the production
# config, and load the StartOnMount LaunchAgent. Never touches
# com.han.external-sync.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_SRC="$SCRIPT_DIR/target/release/exsync"
CONFIG_SRC="$SCRIPT_DIR/exsync.toml"
PLIST_SRC="$SCRIPT_DIR/com.han.exsync.plist"

cargo build --release

mkdir -p "$HOME/.local/bin"
cp "$BIN_SRC" "$HOME/.local/bin/exsync"

mkdir -p "$HOME/Library/Application Support/exsync"
if [ -e "$HOME/Library/Application Support/exsync/exsync.toml" ]; then
  echo "Config already exists at $HOME/Library/Application Support/exsync/exsync.toml; leaving untouched."
  echo "To overwrite: cp \"$CONFIG_SRC\" \"$HOME/Library/Application Support/exsync/exsync.toml\""
else
  cp "$CONFIG_SRC" "$HOME/Library/Application Support/exsync/exsync.toml"
fi

mkdir -p "$HOME/Library/LaunchAgents"
ln -sf "$PLIST_SRC" "$HOME/Library/LaunchAgents/com.han.exsync.plist"

launchctl bootout gui/"$(id -u)"/com.han.exsync 2>/dev/null || true
launchctl bootstrap gui/"$(id -u)" "$HOME/Library/LaunchAgents/com.han.exsync.plist"
