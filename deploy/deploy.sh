#!/usr/bin/env bash
# IronClaw Deployment Script for Multi-Tenant Setup
#
# Usage:
#   ./deploy/deploy.sh                    # Deploy with existing image
#   ./deploy/deploy.sh --build            # Build image locally before deploying
#   ./deploy/deploy.sh --update           # Pull latest and restart

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DOCKER_COMPOSE_FILE="docker-compose.prod.yml"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Parse arguments
BUILD=false
UPDATE=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --build) BUILD=true; shift ;;
        --update) UPDATE=true; shift ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --build    Build Docker image locally before deploying"
            echo "  --update   Pull latest image and restart services"
            echo "  --help     Show this help message"
            exit 0
            ;;
        *) log_error "Unknown option: $1"; exit 1 ;;
    esac
done

cd "$PROJECT_ROOT"

# Check for .env file
if [ ! -f ".env.production" ] && [ ! -f ".env" ]; then
    log_error "Configuration file not found!"
    log_info "Copy .env.production.example to .env.production and configure:"
    log_info "  cp .env.production.example .env.production"
    log_info "  nano .env.production"
    exit 1
fi

# Check for required configuration values
check_required_config() {
    local env_file="${1:-.env.production}"
    local missing=0
    
    # Function to check if a variable is set to a non-placeholder value
    check_var() {
        local var_name="$1"
        local value
        value=$(grep -E "^${var_name}=" "$env_file" 2>/dev/null | cut -d'=' -f2- || true)
        if [ -z "$value" ] || [[ "$value" == *"CHANGE_ME"* ]]; then
            log_warn "$var_name is not set or uses placeholder value"
            missing=$((missing + 1))
        fi
    }
    
    check_var "POSTGRES_PASSWORD"
    check_var "GATEWAY_AUTH_TOKEN"
    
    if [ "$missing" -gt 0 ]; then
        log_error "Missing $missing required configuration value(s)"
        log_info "Edit $env_file and set all CHANGE_ME values"
        exit 1
    fi
    
    # Check for LLM provider
    if ! grep -qE "(OPENAI_API_KEY|ANTHROPIC_API_KEY|NEARAI_API_KEY|LLM_PROVIDER)" "$env_file"; then
        log_warn "No LLM provider configured. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, or NEARAI_API_KEY"
    fi
}

# Validate multi-tenant configuration
validate_multitenant() {
    local env_file="${1:-.env.production}"
    
    if ! grep -q "AGENT_MULTI_TENANT=true" "$env_file" 2>/dev/null; then
        log_warn "AGENT_MULTI_TENANT is not set to 'true'. Multi-tenant mode is DISABLED."
        read -p "Continue in single-user mode? (y/N) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    else
        log_success "Multi-tenant mode is enabled"
    fi
    
    # Validate concurrency limits
    local llm_concurrent
    llm_concurrent=$(grep -E "^TENANT_MAX_LLM_CONCURRENT=" "$env_file" 2>/dev/null | cut -d'=' -f2 || echo "4")
    local jobs_concurrent
    jobs_concurrent=$(grep -E "^TENANT_MAX_JOBS_CONCURRENT=" "$env_file" 2>/dev/null | cut -d'=' -f2 || echo "2")
    
    log_info "Per-user limits: LLM concurrent=$llm_concurrent, Jobs concurrent=$jobs_concurrent"
}

# Build Docker image locally
build_image() {
    log_info "Building IronClaw Docker image..."
    docker build --platform linux/amd64 -t ironclaw/ironclaw:latest .
    log_success "Image built successfully"
}

# Pull latest image
pull_image() {
    local image="${IRONCLAW_IMAGE:-ironclaw/ironclaw:latest}"
    log_info "Pulling $image..."
    docker pull "$image" || log_warn "Failed to pull image, using local if available"
}

# Deploy services
deploy() {
    log_info "Deploying IronClaw services..."
    
    # Use .env.production if it exists, otherwise .env
    local env_file
    if [ -f ".env.production" ]; then
        env_file=".env.production"
    else
        env_file=".env"
    fi
    
    check_required_config "$env_file"
    validate_multitenant "$env_file"
    
    # Stop existing services
    log_info "Stopping existing services..."
    docker compose -f "$DOCKER_COMPOSE_FILE" down --remove-orphans 2>/dev/null || true
    
    # Start services
    log_info "Starting services..."
    docker compose -f "$DOCKER_COMPOSE_FILE" up -d
    
    log_success "Services started!"
    
    # Wait for health check
    log_info "Waiting for services to be healthy..."
    sleep 10
    
    # Check if gateway is responding
    local max_retries=30
    local retry=0
    while [ $retry -lt $max_retries ]; do
        if curl -sf http://localhost:3000/api/health > /dev/null 2>&1; then
            log_success "IronClaw is healthy and ready!"
            break
        fi
        retry=$((retry + 1))
        sleep 2
    done
    
    if [ $retry -eq $max_retries ]; then
        log_warn "Health check timed out. Check logs with:"
        log_warn "  docker compose -f $DOCKER_COMPOSE_FILE logs -f ironclaw"
    fi
    
    # Show status
    echo ""
    echo "========================================"
    echo "IronClaw Multi-Tenant Gateway"
    echo "========================================"
    echo "Gateway:     http://localhost:3000"
    echo "Health:      http://localhost:3000/api/health"
    echo "API Docs:    http://localhost:3000/api"
    echo ""
    echo "Logs:"
    echo "  docker compose -f $DOCKER_COMPOSE_FILE logs -f ironclaw"
    echo ""
    echo "Stop:"
    echo "  docker compose -f $DOCKER_COMPOSE_FILE down"
    echo ""
    echo "Multi-Tenant Status:"
    docker compose -f "$DOCKER_COMPOSE_FILE" ps
}

# Update and restart
update() {
    log_info "Updating IronClaw..."
    pull_image
    deploy
}

# Main execution
main() {
    if [ "$UPDATE" = true ]; then
        update
        exit 0
    fi
    
    if [ "$BUILD" = true ]; then
        build_image
    fi
    
    deploy
}

main