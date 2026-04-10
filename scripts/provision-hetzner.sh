#!/usr/bin/env bash
# provision-hetzner.sh — Idempotent provisioning for Hetzner CCX23 CAS server
# Server: 87.99.156.244 (ubuntu-16gb-ash-1), Ubuntu 24.04, 16GB RAM, 150GB disk
# Usage: ssh root@87.99.156.244 'bash -s' < scripts/provision-hetzner.sh
#
# Secrets (API keys, tokens) are NOT in this script. After running, manually
# populate /home/daniel/.zshrc.secrets (sourced by .zshrc) with:
#   GH_TOKEN, GITHUB_TOKEN, CAS_CLOUD_TOKEN, CAS_CLOUD_ENDPOINT,
#   CONTEXT7_API_KEY, NEON_API_KEY, VERCEL_TOKEN, BROWSERLESS_API_KEY
set -euo pipefail

log() { echo -e "\n==> $1"; }

# ─── 1. System updates & essentials ──────────────────────────────────────────
log "System updates & essentials"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get upgrade -y -qq
apt-get install -y -qq \
  build-essential git curl wget unzip jq htop tmux \
  pkg-config libssl-dev ca-certificates gnupg lld \
  python3 python3-pip python3-venv postgresql-client \
  zsh fzf \
  unattended-upgrades apt-listchanges

# ─── 2. User accounts ────────────────────────────────────────────────────────
create_user() {
  local username="$1"
  if id "$username" &>/dev/null; then
    log "User $username already exists"
  else
    log "Creating user $username"
    adduser --disabled-password --gecos "" "$username"
  fi
  usermod -aG sudo "$username"
  # Passwordless sudo
  echo "$username ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/$username"
  chmod 440 "/etc/sudoers.d/$username"
  # Copy root's authorized_keys if user doesn't have any yet
  local ssh_dir="/home/$username/.ssh"
  mkdir -p "$ssh_dir"
  if [ ! -s "$ssh_dir/authorized_keys" ] && [ -f /root/.ssh/authorized_keys ]; then
    cp /root/.ssh/authorized_keys "$ssh_dir/authorized_keys"
  fi
  chown -R "$username:$username" "$ssh_dir"
  chmod 700 "$ssh_dir"
  chmod 600 "$ssh_dir/authorized_keys"
}

create_user daniel
create_user ben

# ─── 3. Disable root password login ──────────────────────────────────────────
log "Hardening SSH"
sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sshd -t && systemctl reload sshd

# ─── 4. Firewall (UFW) ───────────────────────────────────────────────────────
log "Configuring UFW"
apt-get install -y -qq ufw
ufw allow OpenSSH
ufw --force enable
ufw default deny incoming
ufw default allow outgoing

# ─── 5. Node.js LTS + pnpm ──────────────────────────────────────────────────
log "Installing Node.js LTS"
if ! command -v node &>/dev/null || ! node -v | grep -qE '^v(20|22|24)'; then
  mkdir -p /etc/apt/keyrings
  if [ ! -f /etc/apt/keyrings/nodesource.gpg ]; then
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
      | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg
  fi
  echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_22.x nodistro main" \
    > /etc/apt/sources.list.d/nodesource.list
  apt-get update -qq
  apt-get install -y -qq nodejs
fi
echo "Node.js: $(node -v)"

log "Installing pnpm"
corepack enable
corepack prepare pnpm@latest --activate 2>/dev/null || npm install -g pnpm
echo "pnpm: $(pnpm --version)"

# ─── 6. Claude Code ──────────────────────────────────────────────────────────
log "Installing Claude Code"
if ! command -v claude &>/dev/null; then
  npm install -g @anthropic-ai/claude-code
fi
echo "Claude Code: $(claude --version 2>/dev/null || echo 'installed')"

# ─── 7. Rust toolchain ───────────────────────────────────────────────────────
install_rust_for_user() {
  local username="$1"
  local home_dir="/home/$username"
  if [ -f "$home_dir/.cargo/bin/rustc" ]; then
    local ver
    ver=$(sudo -u "$username" "$home_dir/.cargo/bin/rustc" --version | awk '{print $2}')
    log "Rust already installed for $username: $ver"
  else
    log "Installing Rust for $username"
    sudo -u "$username" bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable'
  fi
}

install_rust_for_user daniel
install_rust_for_user ben

# Also install for root (for building CAS)
if [ ! -f /root/.cargo/bin/rustc ]; then
  log "Installing Rust for root"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
export PATH="/root/.cargo/bin:$PATH"
echo "Rust: $(rustc --version)"

# ─── 8. CAS from source ──────────────────────────────────────────────────────
log "Building CAS from source"
CAS_SRC="/opt/cas-src"
if [ ! -d "$CAS_SRC/.git" ]; then
  git clone https://github.com/pippenz/cas.git "$CAS_SRC"
else
  cd "$CAS_SRC" && git pull --ff-only
fi
cd "$CAS_SRC"
# Bootstrap Zig (required for ghostty_vt_sys)
bash scripts/bootstrap-zig.sh
export ZIG="$CAS_SRC/.context/zig/zig"
/root/.cargo/bin/cargo build --release
ln -sf "$CAS_SRC/target/release/cas" /usr/local/bin/cas
echo "CAS: $(cas --version 2>/dev/null || echo 'built')"

# ─── 9. Swap (4GB) ───────────────────────────────────────────────────────────
log "Configuring swap"
if ! swapon --show | grep -q /swapfile; then
  if [ ! -f /swapfile ]; then
    fallocate -l 4G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile
  fi
  swapon /swapfile
  grep -q '/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi
echo "Swap: $(swapon --show)"

# ─── 10. Sysctl tuning ───────────────────────────────────────────────────────
log "Sysctl tuning"
cat > /etc/sysctl.d/99-cas.conf << 'SYSCTL'
vm.swappiness=10
fs.file-max=1048576
net.core.somaxconn=4096
net.core.netdev_max_backlog=4096
net.ipv4.tcp_max_syn_backlog=4096
net.ipv4.tcp_tw_reuse=1
SYSCTL
sysctl --system -q

# ─── 11. Timezone ────────────────────────────────────────────────────────────
log "Setting timezone to US Eastern"
timedatectl set-timezone America/New_York

# ─── 12. Unattended upgrades ─────────────────────────────────────────────────
log "Configuring unattended upgrades"
cat > /etc/apt/apt.conf.d/20auto-upgrades << 'AUTOUPG'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
AUTOUPG

# ─── 13. CLI tools (eza, starship, zoxide) ───────────────────────────────────
log "Installing CLI tools"

# eza
if ! command -v eza &>/dev/null; then
  mkdir -p /etc/apt/keyrings
  wget -qO- https://raw.githubusercontent.com/eza-community/eza/main/deb.asc \
    | gpg --dearmor -o /etc/apt/keyrings/gierens.gpg
  echo "deb [signed-by=/etc/apt/keyrings/gierens.gpg] http://deb.gierens.de stable main" \
    > /etc/apt/sources.list.d/gierens.list
  chmod 644 /etc/apt/keyrings/gierens.gpg /etc/apt/sources.list.d/gierens.list
  apt-get update -qq
  apt-get install -y -qq eza
fi

# starship
if ! command -v starship &>/dev/null; then
  curl -sS https://starship.rs/install.sh | sh -s -- -y
fi

# zoxide
if ! command -v zoxide &>/dev/null; then
  curl -sS https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh | bash
  cp /root/.local/bin/zoxide /usr/local/bin/zoxide
  chmod 755 /usr/local/bin/zoxide
fi

# ─── 14. Daniel's shell environment ──────────────────────────────────────────
log "Setting up daniel's shell"
chsh -s /usr/bin/zsh daniel

# oh-my-zsh
if [ ! -d /home/daniel/.oh-my-zsh ]; then
  sudo -u daniel sh -c 'RUNZSH=no KEEP_ZSHRC=yes sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)"' || true
fi

# zsh plugins
ZSH_CUSTOM="/home/daniel/.oh-my-zsh/custom"
if [ ! -d "$ZSH_CUSTOM/plugins/zsh-autosuggestions" ]; then
  sudo -u daniel git clone https://github.com/zsh-users/zsh-autosuggestions "$ZSH_CUSTOM/plugins/zsh-autosuggestions"
fi
if [ ! -d "$ZSH_CUSTOM/plugins/zsh-syntax-highlighting" ]; then
  sudo -u daniel git clone https://github.com/zsh-users/zsh-syntax-highlighting "$ZSH_CUSTOM/plugins/zsh-syntax-highlighting"
fi

# .zshrc (secrets loaded from .zshrc.secrets, not committed)
cat > /home/daniel/.zshrc << 'ZSHRC'
# ─── Oh My Zsh ────────────────────────────────────────────────────────────────
export ZSH="$HOME/.oh-my-zsh"
ZSH_THEME=""
plugins=(git zsh-autosuggestions zsh-syntax-highlighting fzf)
source $ZSH/oh-my-zsh.sh

# ─── Starship prompt ─────────────────────────────────────────────────────────
eval "$(starship init zsh)"

# ─── Rust / Cargo ─────────────────────────────────────────────────────────────
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# ─── NVM ──────────────────────────────────────────────────────────────────────
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && source "$NVM_DIR/nvm.sh"
[ -s "$NVM_DIR/bash_completion" ] && source "$NVM_DIR/bash_completion"

# ─── eza aliases ──────────────────────────────────────────────────────────────
alias ls="eza --icons"
alias ll="eza -la --icons --git"
alias la="eza -a --icons"
alias lt="eza --tree --icons -L 3"

# ─── Zoxide / fzf ────────────────────────────────────────────────────────────
eval "$(zoxide init zsh)"
[ -f /usr/share/doc/fzf/examples/key-bindings.zsh ] && source /usr/share/doc/fzf/examples/key-bindings.zsh
[ -f /usr/share/doc/fzf/examples/completion.zsh ] && source /usr/share/doc/fzf/examples/completion.zsh

# ─── Aliases ──────────────────────────────────────────────────────────────────
alias python=python3

# ─── CAS wrapper ──────────────────────────────────────────────────────────────
cas() {
  if [ $# -eq 0 ]; then
    command cas factory --new
  else
    command cas "$@"
  fi
}

cas-refresh() {
  cd ~/cas-src && git pull --ff-only && cargo build --release \
    && sudo ln -sf ~/cas-src/target/release/cas /usr/local/bin/cas && cas --version
}

# ─── SSH agent persistent socket ──────────────────────────────────────────────
export SSH_AUTH_SOCK="$HOME/.ssh/agent.sock"
if ! ss -a | grep -q "$SSH_AUTH_SOCK"; then
  rm -f "$SSH_AUTH_SOCK"
  eval $(ssh-agent -a "$SSH_AUTH_SOCK") > /dev/null
fi

# ─── Secrets (not in repo — populate after provisioning) ─────────────────────
[ -f "$HOME/.zshrc.secrets" ] && source "$HOME/.zshrc.secrets"

# ─── CAS login alias + auto-login ────────────────────────────────────────────
alias cas-login='cas login --token $CAS_CLOUD_TOKEN --endpoint $CAS_CLOUD_ENDPOINT'
cas auth whoami &>/dev/null 2>&1 || cas login --token "$CAS_CLOUD_TOKEN" --endpoint "$CAS_CLOUD_ENDPOINT" &>/dev/null 2>&1 || true

# ─── Terminal title ───────────────────────────────────────────────────────────
precmd() { print -Pn "\e]0;%n@%m:%~\a"; }
ZSHRC
chown daniel:daniel /home/daniel/.zshrc

# ─── 15. Per-user directories ────────────────────────────────────────────────
setup_user_dirs() {
  local username="$1"
  local home_dir="/home/$username"
  log "Setting up directories for $username"
  sudo -u "$username" mkdir -p \
    "$home_dir/.cas" \
    "$home_dir/.claude" \
    "$home_dir/projects"
}

setup_user_dirs daniel
setup_user_dirs ben

# ─── Done ─────────────────────────────────────────────────────────────────────
log "Provisioning complete!"
echo ""
echo "Summary:"
echo "  Users: daniel (sudo+key+zsh), ben (sudo, no key yet)"
echo "  Node.js: $(node -v), pnpm: $(pnpm --version)"
echo "  Python: $(python3 --version)"
echo "  Claude Code: $(claude --version 2>/dev/null || echo 'installed')"
echo "  Rust: $(rustc --version)"
echo "  CAS: $(cas --version 2>/dev/null || echo 'built')"
echo "  UFW: $(ufw status | head -1)"
echo "  Swap: $(swapon --show --noheadings | awk '{print $3}')"
echo "  Timezone: $(timedatectl show -p Timezone --value)"
echo ""
echo "Next steps:"
echo "  - Populate /home/daniel/.zshrc.secrets with API tokens"
echo "  - Add ben's SSH public key to /home/ben/.ssh/authorized_keys"
echo "  - SSH as daniel: ssh daniel@87.99.156.244"
