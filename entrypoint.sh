#!/bin/bash
set -e

# Set log level for debugging
export RUST_LOG="${RUST_LOG:-info}"

# Validate required environment variables (DB or API mode)
if [ -n "$DB_USERNAME" ] && [ -n "$DB_PASSWORD" ] && [ -n "$DB_DATABASE_NAME" ]; then
    # Database mode
    :
elif [ -n "$IMMICH_API_URL" ] && [ -n "$IMMICH_API_KEY" ]; then
    # API mode
    :
else
    echo "ERROR: Either (IMMICH_API_URL + IMMICH_API_KEY) OR (DB_USERNAME + DB_PASSWORD + DB_DATABASE_NAME) must be set"
    exit 1
fi

# Set default values for optional database connection variables
DB_HOSTNAME="${DB_HOSTNAME:-database}"
DB_PORT="${DB_PORT:-5432}"

# Build safe arguments array
args=(
    "--combined"
)

# Add data access mode
if [ -n "$DB_USERNAME" ] && [ -n "$DB_PASSWORD" ] && [ -n "$DB_DATABASE_NAME" ]; then
    args+=("--data-access-mode" "database")
    args+=("--postgres-url" "postgresql://$DB_USERNAME:$DB_PASSWORD@$DB_HOSTNAME:$DB_PORT/$DB_DATABASE_NAME")
    args+=("--immich-root" "/data")
else
    args+=("--data-access-mode" "immich-api")
    # immich_api_url/immich_api_key are read from env by clap - no need to pass explicitly
fi

# Add optional configuration safely
if [ -n "$IMMICH_ANALYZE_INTERFACE" ]; then
    args+=("--interface" "$IMMICH_ANALYZE_INTERFACE")
fi

if [ -n "$IMMICH_ANALYZE_HOSTS" ]; then
    args+=("--hosts" "$IMMICH_ANALYZE_HOSTS")
elif [ -n "$IMMICH_ANALYZE_OLLAMA_HOSTS" ]; then
    # Backwards compatibility
    args+=("--hosts" "$IMMICH_ANALYZE_OLLAMA_HOSTS")
fi

# api_key are read from env by clap - no need to pass explicitly

if [ -n "$IMMICH_ANALYZE_MODEL_NAME" ]; then
    args+=("--model-name" "$IMMICH_ANALYZE_MODEL_NAME")
fi

if [ -n "$IMMICH_ANALYZE_PROMPT" ]; then
    args+=("--prompt" "$IMMICH_ANALYZE_PROMPT")
fi

if [ "${IMMICH_ANALYZE_OVERWRITE_EXISTING:-false}" = "true" ]; then
    args+=("--overwrite-existing")
fi

if [ -n "$IMMICH_ANALYZE_LANG" ]; then
    args+=("--lang" "$IMMICH_ANALYZE_LANG")
fi

# Numeric validations
if [[ "$IMMICH_ANALYZE_MAX_CONCURRENT" =~ ^[0-9]+$ ]]; then
    args+=("--max-concurrent" "$IMMICH_ANALYZE_MAX_CONCURRENT")
fi

if [[ "$IMMICH_ANALYZE_UNAVAILABLE_DURATION" =~ ^[0-9]+$ ]]; then
    args+=("--unavailable-duration" "$IMMICH_ANALYZE_UNAVAILABLE_DURATION")
fi

if [[ "$IMMICH_ANALYZE_TIMEOUT" =~ ^[0-9]+$ ]]; then
    args+=("--timeout" "$IMMICH_ANALYZE_TIMEOUT")
fi

if [[ "$IMMICH_ANALYZE_API_POLL_INTERVAL" =~ ^[0-9]+$ ]]; then
    args+=("--api-poll-interval" "$IMMICH_ANALYZE_API_POLL_INTERVAL")
fi

echo "Running immich-analyze with args: ${args[@]}"

# Execute with proper signal handling
exec immich-analyze "${args[@]}"
