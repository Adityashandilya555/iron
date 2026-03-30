-- IronClaw PostgreSQL initialization script
-- Creates necessary extensions and schema for multi-tenant deployment

-- Enable pgvector extension for embeddings support
CREATE EXTENSION IF NOT EXISTS vector;

-- Enable UUID generation
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Create indexes for performance (if not exists)
-- These are created by migrations but we ensure vector index exists

-- Note: Actual table creation is handled by IronClaw's migration system
-- This script just ensures extensions are available