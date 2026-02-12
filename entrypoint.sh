#!/bin/bash
set -e

# Set log level for debugging
export RUST_LOG="${RUST_LOG:-info}"

# Validate required environment variables
required_vars=(
    "DB_USERNAME"
    "DB_PASSWORD"
    "DB_DATABASE_NAME"
)

for var in "${required_vars[@]}"; do
    if [ -z "${!var}" ]; then
        echo "ERROR: Required environment variable $var is not set"
        exit 1
    fi
done

# Set default values for optional database connection variables
DB_HOSTNAME="${DB_HOSTNAME:-database}"
DB_PORT="${DB_PORT:-5432}"

# Build safe arguments array
args=(
    "--combined"
    "--immich-root" "/data"
    "--postgres-url" "postgresql://$DB_USERNAME:$DB_PASSWORD@$DB_HOSTNAME:$DB_PORT/$DB_DATABASE_NAME"
)

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

if [ -n "$IMMICH_ANALYZE_API_KEY" ]; then
    args+=("--api-key" "$IMMICH_ANALYZE_API_KEY")
fi

if [ -n "$IMMICH_ANALYZE_MODEL_NAME" ]; then
    args+=("--model-name" "$IMMICH_ANALYZE_MODEL_NAME")
fi

if [ -n "$IMMICH_ANALYZE_PROMPT" ]; then
    args+=("--prompt" "$IMMICH_ANALYZE_PROMPT")
fi

if [ "${IMMICH_ANALYZE_IGNORE_EXISTING:-false}" = "true" ]; then
    args+=("--ignore-existing")
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

echo "Running immich-analyze with args: ${args[@]}"

# Execute with proper signal handling
exec immich-analyze "${args[@]}"
