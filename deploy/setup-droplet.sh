#!/usr/bin/env bash
# DigitalOcean Droplet Setup Script for IronClaw Multi-Tenant Deployment
#
# Run this script on a fresh Ubuntu 22.04/24.04 droplet:
#   curl -fsSL https://raw.githubusercontent.com/Adityashandilya555/iron/main/deploy/setup-droplet.sh | sudo bash
#
# Prerequisites:
#   - Ubuntu 22.04 or 24.04 LTS
#   - Root or sudo access
#
# What this script installs:
#   - Docker CE with Docker Compose
#   - UFW firewall (ports 22, 80, 443, 3000)
#   - System tuning for production
#   - Swap file (if < 4GB RAM)
#   - Unattended security upgrades

set -euo pipefail

echo "========================================"
echo "IronClaw Multi-Tenant Server Setup"
echo "========================================"

# Check for root
if [ "$(id -u)" -ne 0 ]; then
    echo "ERROR: Run with sudo: sudo bash $0"
    exit 1
fi

# Detect OS
if [ ! -f /etc/lsb-release ] || ! grep -q "Ubuntu" /etc/lsb-release; then
    echo "WARNING: This script is tested on Ubuntu 22.04/24.04"
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

echo ""
echo "==> System Update"
apt-get update
apt-get upgrade -y

echo ""
echo "==> Installing Docker CE"
# Add Docker's official GPG key
curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker.gpg

# Set up Docker repository
echo \
  "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu \
  $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
  tee /etc/apt/sources.list.d/docker.list > /dev/null

apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

# Enable Docker
systemctl enable --now docker

# Add current user to docker group if not root
if [ -n "$SUDO_USER" ] && [ "$SUDO_USER" != "root" ]; then
    usermod -aG docker "$SUDO_USER"
    echo "Added $SUDO_USER to docker group. Log out and back in for this to take effect."
fi

echo ""
echo "==> Configuring Firewall (UFW)"
# Reset UFW to clean state
ufw --force reset

# Default policies
ufw default deny incoming
ufw default allow outgoing

# Allow essential ports
ufw allow 22/tcp comment "SSH"
ufw allow 80/tcp comment "HTTP"
ufw allow 443/tcp comment "HTTPS"
ufw allow 3000/tcp comment "IronClaw Gateway"

# Enable firewall
ufw --force enable

echo ""
echo "==> System Tuning"
# Increase file descriptor limits
cat > /etc/security/limits.d/ironclaw.conf << 'EOF'
* soft nofile 65536
* hard nofile 65536
root soft nofile 65536
root hard nofile 65536
EOF

# Kernel tuning for high connections
cat > /etc/sysctl.d/99-ironclaw.conf << 'EOF'
# Network tuning
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.core.netdev_max_backlog = 65535
net.ipv4.tcp_fin_timeout = 10
net.ipv4.tcp_tw_reuse = 1
net.ipv4.ip_local_port_range = 1024 65535

# Memory
vm.swappiness = 10
EOF
sysctl -p /etc/sysctl.d/99-ironclaw.conf

echo ""
echo "==> Swap Setup (if needed)"
TOTAL_MEM=$(grep MemTotal /proc/meminfo | awk '{print $2}')
if [ "$TOTAL_MEM" -lt 4000000 ]; then
    echo "System has < 4GB RAM, creating 2GB swap..."
    fallocate -l 2G /swapfile || dd if=/dev/zero of=/swapfile bs=1M count=2048
    chmod 600 /swapfile
    mkswap /swapfile
    swapon /swapfile
    echo '/swapfile none swap sw 0 0' >> /etc/fstab
else
    echo "System has sufficient RAM, skipping swap creation"
fi

echo ""
echo "==> Security Updates"
apt-get install -y unattended-upgrades
dpkg-reconfigure --priority=low unattended-upgrades

echo ""
echo "==> Creating IronClaw Directories"
mkdir -p /opt/ironclaw/config
mkdir -p /opt/ironclaw/data
mkdir -p /opt/ironclaw/logs
mkdir -p /var/lib/ironclaw/workspaces
mkdir -p /var/lib/ironclaw/skills

# Set permissions for ironclaw user (if exists) or root
if id -u ironclaw &>/dev/null; then
    chown -R ironclaw:ironclaw /opt/ironclaw /var/lib/ironclaw
else
    echo "NOTE: 'ironclaw' user not created yet. Docker container runs as uid 1000."
    echo "      Directories owned by root - Docker will handle permissions."
fi

echo ""
echo "==> Clone IronClaw Repository"
if [ ! -d /opt/ironclaw/repo ]; then
    apt-get install -y git
    git clone https://github.com/Adityashandilya555/iron.git /opt/ironclaw/repo || {
        echo "Failed to clone repository. Clone manually:"
        echo "  git clone https://github.com/Adityashandilya555/iron.git /opt/ironclaw/repo"
    }
fi

echo ""
echo "========================================"
echo "Setup Complete!"
echo "========================================"
echo ""
echo "Next Steps:"
echo "-----------"
echo "1. Copy your configuration:"
echo "   cp /opt/ironclaw/repo/.env.production.example /opt/ironclaw/.env"
echo ""
echo "2. Edit configuration:"
echo "   nano /opt/ironclaw/.env"
echo "   # Set: POSTGRES_PASSWORD, GATEWAY_AUTH_TOKEN, LLM provider keys"
echo ""
echo "3. Start IronClaw:"
echo "   cd /opt/ironclaw/repo"
echo "   docker compose -f docker-compose.prod.yml up -d"
echo ""
echo "4. Check logs:"
echo "   docker compose -f docker-compose.prod.yml logs -f ironclaw"
echo ""
echo "5. Health check:"
echo "   curl http://localhost:3000/api/health"
echo ""
echo "Firewall ports:"
echo "  22   - SSH"
echo "  80   - HTTP (for reverse proxy)"
echo "  443  - HTTPS (for reverse proxy)"
echo "  3000 - IronClaw Gateway (exposed for direct access or reverse proxy)"