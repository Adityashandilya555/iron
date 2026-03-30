# Multi-Tenant Deployment Guide

This guide covers deploying IronClaw in multi-tenant mode for serving multiple concurrent users on a DigitalOcean droplet (or similar cloud provider).

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        DIGITALOCEAN DROPLET                          │
│                                                                      │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐  │
│  │   IronClaw      │    │   PostgreSQL    │    │   (Optional)    │  │
│  │   Container     │───▶│   + pgvector    │    │     Redis       │  │
│  │                 │    │                 │    │                 │  │
│  │ - TenantScope  │    │ - Conversations │    │ - Rate Limit    │  │
│  │ - WorkspacePool │    │ - Jobs          │    │   (future)      │  │
│  │ - CostGuard     │    │ - Settings      │    │                 │  │
│  │ - PerUserSemaph │    │ - Embeddings    │    │                 │  │
│  └────────┬────────┘    └─────────────────┘    └─────────────────┘  │
│           │                                                         │
│           ▼                                                         │
│  ┌─────────────────┐                                                │
│  │ Web Gateway     │◀──── Telegram Bot API (webhook/polling)       │
│  │ :3000           │◀──── Discord Gateway                           │
│  │                 │◀──── HTTP API (apps)                           │
│  └─────────────────┘                                                │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘

Multi-Tenant Isolation:
┌──────────────┬──────────────┬──────────────────────────────────────┐
│ User Alice   │ User Bob     │ User Charlie                         │
├──────────────┼──────────────┼───────────────────────────────────────┤
│ tenant_id:   │ tenant_id:   │ tenant_id:                           │
│ telegram:123 │ telegram:456 │ telegram:789                         │
├──────────────┼──────────────┼───────────────────────────────────────┤
│ Semaphore:   │ Semaphore:   │ Semaphore:                           │
│ LLM: 2/4     │ LLM: 1/4     │ LLM: 0/4 (waiting)                    │
│ Jobs: 1/2    │ Jobs: 0/2    │ Jobs: 0/2                            │
├──────────────┼──────────────┼───────────────────────────────────────┤
│ Workspace:   │ Workspace:   │ Workspace:                           │
│ /alice/      │ /bob/        │ /charlie/                            │
└──────────────┴──────────────┴───────────────────────────────────────┘

TenantScope filters ALL database queries by user_id automatically.
```

## Capacity Planning

### Recommended Droplet Sizes

| Users | Concurrent | Droplet | RAM | vCPUs | Storage | Est. Cost/mo |
|-------|------------|---------|-----|-------|---------|--------------|
| 1-10 | 2-5 | Basic | 2GB | 1 vCPU | 50GB SSD | $12 |
| 10-50 | 5-20 | Standard | 4GB | 2 vCPU | 80GB SSD | $24 |
| 50-200 | 20-50 | Standard | 8GB | 4 vCPU | 160GB SSD | $48 |
| 200-500 | 50-100 | Performance | 16GB | 8 vCPU | 320GB SSD | $96 |

### Concurrency Math

With `TENANT_MAX_LLM_CONCURRENT=4` and `20 vCPUs`:
- Theoretical max: `20 / 4 = 5` users processing LLM concurrently
- Practical: ~10-15 users with interleaved I/O (waiting for LLM responses)
- With `TENANT_MAX_LLM_CONCURRENT=2`: ~20-30 concurrent users

**Key insight**: LLM calls are I/O-bound (waiting for API). While one user's request is at the LLM API, other users can start processing.

### Expected Latency

| Factor | Impact |
|--------|--------|
| LLM API latency | 1-5 seconds (depends on provider) |
| Database query | <50ms (same region) |
| Internal processing | <100ms |
| Queue wait time | Variable (see concurrency limits) |

**Low-latency scenario** (same region as LLM API, light load):
- Response time: 1-3 seconds

**High-load scenario** (many concurrent users):
- Users beyond semaphore limit queue
- Typical wait: +5-10 seconds per queued request

## Deployment Steps

### 1. Create Droplet

```bash
# Via DigitalOcean Dashboard:
# - Ubuntu 24.04 LTS
# - Choose size based on expected users (see table above)
# - Add SSH key
# - Enable monitoring

# Or via doctl CLI:
doctl compute droplet create ironclaw \
  --region nyc1 \
  --size s-4vcpu-8gb \
  --image ubuntu-24-04-x64 \
  --ssh-keys YOUR_SSH_KEY_ID
```

### 2. Run Setup Script

```bash
# SSH into droplet
ssh root@YOUR_DROPLET_IP

# Download and run setup script
curl -fsSL https://raw.githubusercontent.com/Adityashandilya555/iron/main/deploy/setup-droplet.sh | sudo bash
```

### 3. Configure Environment

```bash
# Copy example config
cp /opt/ironclaw/repo/.env.production.example /opt/ironclaw/.env

# Edit configuration
nano /opt/ironclaw/.env
```

**Critical settings for multi-tenant:**

```env
# MUST be true for multi-user
AGENT_MULTI_TENANT=true

# Per-user limits
TENANT_MAX_LLM_CONCURRENT=4
TENANT_MAX_JOBS_CONCURRENT=2

# Auth (for Telegram/Discord bots, use single token)
# For multi-user via web, configure AUTH_TOKENS or use database auth
GATEWAY_AUTH_TOKEN=your-secure-random-token-here

# LLM Provider (choose one)
OPENAI_API_KEY=sk-xxx
# or ANTHROPIC_API_KEY=sk-ant-xxx
# or NEARAI_API_KEY=xxx
```

### 4. Deploy

```bash
cd /opt/ironclaw/repo
docker compose -f docker-compose.prod.yml up -d
```

### 5. Verify

```bash
# Check health
curl http://localhost:3000/api/health

# Check logs
docker compose -f docker-compose.prod.yml logs -f ironclaw

# Verify multi-tenant in logs:
# Look for: "multi_tenant=true" in startup output
```

## Telegram Bot Setup

For Telegram as the user interface:

```env
# In .env
TELEGRAM_BOT_TOKEN=123456789:ABCDEF...
# No TELEGRAM_OWNER_ID for multi-user (open to all who have bot access)
```

The Telegram channel automatically:
1. Extracts `user_id` from `telegram:CHAT_ID` or `telegram:USER_ID`
2. Creates isolated sessions per Telegram user
3. Each user gets their own workspace and conversation history

### Group Chat Behavior

In Telegram groups:
- Bot responds when **@mentioned**
- Or set `TELEGRAM_RESPOND_TO_ALL_GROUP_MESSAGES=true` (spammy!)

User identity extraction:
- Private chat: `telegram:{user_id}`
- Group chat (mentioned): `telegram:{mentioned_user_id}`
- Group chat (all messages): `telegram:{sender_user_id}`

## Monitoring

### Resource Usage

```bash
# Container stats
docker stats ironclaw

# PostgreSQL stats
docker exec -it ironclaw-postgres psql -U ironclaw -c "
SELECT 
  datname,
  pg_size_pretty(pg_database_size(datname)) as size
FROM pg_database
WHERE datname = 'ironclaw';
"

# Active connections
docker exec -it ironclaw-postgres psql -U ironclaw -c "
SELECT count(*) as active_connections FROM pg_stat_activity;
"
```

### Log Aggregation

```bash
# Export logs to file
docker compose -f docker-compose.prod.yml logs --no-color > /var/log/ironclaw/$(date +%Y%m%d).log

# Or use Docker logging drivers for external aggregation
```

## Scaling Beyond Single Droplet

When you exceed single-node capacity:

### Horizontal Scaling (Multiple Droplets)

```
┌──────────────────────────────────────────────────────────────────┐
│                        Load Balancer                             │
│                   (DigitalOcean LB or Caddy)                     │
└──────────────────────────────────────────────────────────────────┘
         │                    │                    │
         ▼                    ▼                    ▼
┌────────────────┐   ┌────────────────┐   ┌────────────────┐
│ IronClaw Node 1│   │ IronClaw Node 2│   │ IronClaw Node 3│
│ (Stateless)    │   │ (Stateless)    │   │ (Stateless)    │
└───────┬────────┘   └───────┬────────┘   └───────┬────────┘
        │                    │                    │
        └────────────────────┼────────────────────┘
                             ▼
                  ┌─────────────────────┐
                  │   Managed Postgres  │
                  │   (DigitalOcean)    │
                  │   + Redis (future)  │
                  └─────────────────────┘
```

**Steps:**
1. Use DigitalOcean Managed PostgreSQL (offloads DB management)
2. Deploy multiple IronClaw droplets with same config
3. Add load balancer in front
4. Sticky sessions (for WebSocket/SSE continuity)

**Note**: Current architecture requires sticky sessions. Future versions may support fully stateless horizontal scaling.

## Troubleshooting

### "Multi-tenant mode is not enabled"

Check `.env` contains:
```env
AGENT_MULTI_TENANT=true
```

Restart after changing:
```bash
docker compose -f docker-compose.prod.yml restart
```

### Users getting each other's context

1. Verify `AGENT_MULTI_TENANT=true` in logs at startup
2. Check `TenantScope` is being used (all DB queries go through it)
3. Check workspace isolation in logs: `workspace=/var/lib/ironclaw/workspaces/telegram:123`

### Rate limiting not working per-user

Verify per-user semaphores in logs:
```
[INFO] Created TenantRateState for user telegram:123 (llm_max=4, jobs_max=2)
```

### High memory usage

Each user workspace consumes memory. Reduce `SESSION_IDLE_TIMEOUT_SECS` to clean up idle sessions faster:
```env
SESSION_IDLE_TIMEOUT_SECS=1800  # 30 minutes instead of 1 hour
```

### Database connection exhaustion

Check PostgreSQL limit:
```bash
# Current connections
docker exec ironclaw-postgres psql -U ironclaw -c "SHOW max_connections;"

# Increase if needed (edit postgresql.conf)
max_connections = 200
```

## Security Checklist

- [ ] `POSTGRES_PASSWORD` is strong and unique
- [ ] `GATEWAY_AUTH_TOKEN` is random (32+ chars)
- [ ] LLM API keys are configured but not in git
- [ ] UFW firewall enabled (ports 22, 80, 443, 3000)
- [ ] SSH key-only authentication (password disabled)
- [ ] `.env` file permissions: `chmod 600 /opt/ironclaw/.env`
- [ ] Docker container runs as non-root user (uid 1000)
- [ ] HTTPS configured (via reverse proxy)