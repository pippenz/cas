#!/usr/bin/env bash
# CAS Slack Bridge — Server Setup Script
#
# Run as root on the CAS server to:
# 1. Create the cas-bridge system user
# 2. Install systemd services
# 3. Create config directories
# 4. Build and deploy the bridge code
#
# Usage: sudo bash setup.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=== CAS Slack Bridge Setup ==="

# 1. Create system user for the router
if ! id cas-bridge &>/dev/null; then
    useradd --system --shell /usr/sbin/nologin --no-create-home cas-bridge
    echo "Created cas-bridge system user"
else
    echo "cas-bridge user already exists"
fi

# 2. Create directories
mkdir -p /etc/cas-bridge
mkdir -p /run/cas-bridge
mkdir -p /opt/cas-bridge
chown cas-bridge:cas-bridge /run/cas-bridge
chmod 755 /run/cas-bridge
echo "Created directories"

# 3. Install config if not present
if [ ! -f /etc/cas-bridge/config.json ]; then
    cp "$SCRIPT_DIR/config.example.json" /etc/cas-bridge/config.json
    chmod 644 /etc/cas-bridge/config.json
    echo "Installed example config to /etc/cas-bridge/config.json — EDIT THIS"
else
    echo "Config already exists at /etc/cas-bridge/config.json"
fi

if [ ! -f /etc/cas-bridge/router.env ]; then
    cp "$SCRIPT_DIR/router.env.example" /etc/cas-bridge/router.env
    chmod 600 /etc/cas-bridge/router.env
    chown cas-bridge:cas-bridge /etc/cas-bridge/router.env
    echo "Installed example router.env to /etc/cas-bridge/router.env — EDIT THIS"
else
    echo "Router env already exists at /etc/cas-bridge/router.env"
fi

# 4. Build TypeScript
echo "Building TypeScript..."
cd "$SCRIPT_DIR"
npm install
npm run build

# 5. Deploy built code
cp -r dist/ /opt/cas-bridge/dist/
cp package.json /opt/cas-bridge/
cd /opt/cas-bridge && npm install --omit=dev
echo "Deployed to /opt/cas-bridge"

# 6. Install systemd services
cp "$SCRIPT_DIR/systemd/cas-bridge-router.service" /etc/systemd/system/
cp "$SCRIPT_DIR/systemd/cas-bridge@.service" /etc/systemd/system/
cp "$SCRIPT_DIR/systemd/cas-bridge.tmpfiles" /etc/tmpfiles.d/cas-bridge.conf
systemd-tmpfiles --create
systemctl daemon-reload
echo "Installed systemd services"

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "  1. Edit /etc/cas-bridge/config.json — add channel IDs and Slack user IDs"
echo "  2. Edit /etc/cas-bridge/router.env — add SLACK_BOT_TOKEN and SLACK_APP_TOKEN"
echo "  3. For each user, create ~/.config/cas/env with CAS_SERVE_URL, CAS_SERVE_TOKEN, SLACK_BOT_TOKEN"
echo "  4. Start the router:   sudo systemctl enable --now cas-bridge-router"
echo "  5. Start user daemons: sudo systemctl enable --now cas-bridge@daniel cas-bridge@ben"
echo "  6. Check logs:         journalctl -u cas-bridge-router -f"
echo "                         journalctl -u cas-bridge@daniel -f"
