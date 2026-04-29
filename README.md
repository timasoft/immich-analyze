# Immich Analyze

AI-powered image description generator for Immich photo management system

## Overview

Immich Analyze automatically generates detailed descriptions for images in your Immich library using AI vision models via **Ollama** or **llama.cpp server**. This enhances search capabilities and organization by providing semantic understanding of image content.

The application supports two data access modes:
- **Database mode**: Direct PostgreSQL database access for reading/writing Immich metadata
- **API mode**: Uses the Immich API (requires `IMMICH_API_URL` + `IMMICH_API_KEY`)

## Features

- AI-powered image analysis using Ollama or llama.cpp server with vision-capable models
- Multiple operation modes: batch processing, folder monitoring, or combined mode
- Multi-host support with automatic failover for AI service endpoints
- **Dual data access modes**: Direct PostgreSQL database access OR Immich API integration
- Concurrent processing with configurable parallelism
- Configurable retry logic with max retries and delay between attempts
- Internationalization support (English and Russian)
- Docker container support
- Structured logging via `env_logger` (configure with `RUST_LOG` environment variable)

## Prerequisites

- Immich instance with either:
  - PostgreSQL database access, OR
  - Immich API endpoint with API key
- AI service running a vision-capable model:
  - **Ollama** server (e.g., `qwen3-vl:4b-thinking-q4_K_M`), OR
  - **llama.cpp server** with OpenAI-compatible API endpoint

## Installation

### Docker Compose Integration (Recommended)

To integrate Immich Analyze directly into your Immich setup, add the following service to your `docker-compose.yaml` file:

```yaml
services:
  # Optional: Ollama service (you can use external Ollama or llama.cpp server instead)
  # This section is optional - remove it if you want to use external AI service
  ollama:
    image: ollama/ollama:latest
    container_name: ollama
    restart: unless-stopped
    ports:
      - "11434:11434"
    volumes:
      - ./ollama:/root/.ollama
    networks:
      - immich-network
    # Optional: GPU acceleration for NVIDIA cards
    # deploy:
    #   resources:
    #     reservations:
    #       devices:
    #         - driver: nvidia
    #           count: 1
    #           capabilities: [gpu]

  immich-analyze:
    image: ghcr.io/timasoft/immich-analyze:main
    container_name: immich-analyze
    restart: unless-stopped
    volumes:
      # Only required for database mode (to access /data/upload, /data/thumbs)
      - ${UPLOAD_LOCATION}:/data
      - /etc/localtime:/etc/localtime:ro
    env_file:
      - .env
    environment:
      # AI service configuration
      - IMMICH_ANALYZE_INTERFACE=ollama  # or "llamacpp"
      - IMMICH_ANALYZE_HOSTS=http://ollama:11434
      # For llama.cpp server with authentication:
      # - IMMICH_ANALYZE_INTERFACE=llamacpp
      # - IMMICH_ANALYZE_HOSTS=http://llamacpp-server:8080
      # - IMMICH_ANALYZE_API_KEY=your-api-key-here
      # Or use multiple hosts with automatic failover:
      # - IMMICH_ANALYZE_HOSTS=http://primary:11434,http://backup:11434
    depends_on:
      - database
      # Comment the next line if using external AI service
      - ollama
    networks:
      - immich-network

networks:
  immich-network:
    external: true
```

**Important notes about configuration:**

- **Data Access Mode**: You must provide EITHER:
  - Database credentials (`DB_USERNAME`, `DB_PASSWORD`, `DB_DATABASE_NAME`) for direct PostgreSQL access, OR
  - API credentials (`IMMICH_API_URL`, `IMMICH_API_KEY`) for Immich API access
- **Volume mounts**: The `/data` volume mount is only required when using **database mode** (to access `upload/` and `thumbs/` directories). When using **API mode**, this volume can be omitted.
- The `ollama` service is **optional** - you can remove it and use an external Ollama or llama.cpp server instead
- Set `IMMICH_ANALYZE_INTERFACE` to `ollama` (default) or `llamacpp` depending on your backend
- If using external service, modify `IMMICH_ANALYZE_HOSTS` to point to your server(s)
- For llama.cpp server, provide `IMMICH_ANALYZE_API_KEY` if authentication is enabled
- After adding the Ollama service, you need to pull the model manually by executing:
  ```bash
  docker exec -it ollama ollama pull qwen3-vl:4b-thinking-q4_K_M
  ```
- For GPU acceleration with NVIDIA cards, uncomment the deploy section and ensure you have NVIDIA Container Toolkit installed

Make sure to:
1. Add the service(s) to your existing `docker-compose.yml` file
2. Ensure the `immich-network` exists or create a new network
3. Add the required environment variables to your `.env` file

After adding the service, run:
```bash
docker-compose up -d immich-analyze
# If using internal Ollama service:
# docker-compose up -d ollama
```

### Nix

If you're using Nix or NixOS, you can build and run the application directly:

**Database mode:**
```bash
nix run github:timasoft/immich-analyze -- --data-access-mode database --immich-root /path/to/immich/data --postgres-url "host=localhost user=your_postgres_user dbname=immich password=your_postgres_password" -c
```

**API mode:**
```bash
IMMICH_API_URL=http://localhost:2283 IMMICH_API_KEY=your_key nix run github:timasoft/immich-analyze -- --data-access-mode immich-api -c
```

### From Source

1. Install Rust toolchain:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Install the project:
   ```bash
   cargo install immich-analyze
   ```

3. Run the application:

   **Database mode:**
   ```bash
   immich-analyze --data-access-mode database --immich-root /path/to/immich/data --postgres-url "host=localhost user=your_postgres_user dbname=immich password=your_postgres_password" -c
   ```

   **API mode:**
   ```bash
   IMMICH_API_URL=http://localhost:2283 IMMICH_API_KEY=your_key immich-analyze --data-access-mode immich-api -c
   ```

## Configuration

### Environment Variables (Docker)

#### Data Access Configuration (choose ONE mode)

| Variable | Description | Default | Required For |
|----------|-------------|---------|-------------|
| `DB_USERNAME` | PostgreSQL username | - | Database mode |
| `DB_PASSWORD` | PostgreSQL password | - | Database mode |
| `DB_DATABASE_NAME` | PostgreSQL database name | - | Database mode |
| `DB_HOSTNAME` | PostgreSQL hostname | `database` | Database mode |
| `DB_PORT` | PostgreSQL port | `5432` | Database mode |
| `IMMICH_API_URL` | Immich API base URL | - | API mode |
| `IMMICH_API_KEY` | Immich API authentication key | - | API mode |

#### AI Service Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `IMMICH_ANALYZE_INTERFACE` | AI service interface type (`ollama` or `llamacpp`) | `ollama` |
| `IMMICH_ANALYZE_HOSTS` | Comma-separated AI service host URLs | `http://localhost:11434` |
| `IMMICH_ANALYZE_API_KEY` | API key for llama.cpp server authentication | *(none)* |
| `IMMICH_ANALYZE_MODEL_NAME` | Model name for image analysis | `qwen3-vl:4b-thinking-q4_K_M` |
| `IMMICH_ANALYZE_PROMPT` | Prompt for generating image descriptions | *See below* |
| `IMMICH_ANALYZE_API_POLL_INTERVAL` | Poll interval for API mode in seconds | `10` |

#### Application Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `IMMICH_ANALYZE_OVERWRITE_EXISTING` | If true, overwrite existing descriptions | `false` |
| `IMMICH_ANALYZE_LANG` | Interface language for the application (en, ru) | `en` |
| `IMMICH_ANALYZE_MAX_CONCURRENT` | Max concurrent AI requests | `4` |
| `IMMICH_ANALYZE_UNAVAILABLE_DURATION` | Host availability check interval in seconds | `60` |
| `IMMICH_ANALYZE_TIMEOUT` | AI request timeout in seconds | `300` |
| `IMMICH_ANALYZE_MAX_RETRIES` | Maximum retry attempts (0 = infinite) | `0` |
| `IMMICH_ANALYZE_RETRY_DELAY_SECONDS` | Delay between retry cycles in seconds | `5` |
| `RUST_LOG` | Logging level (`error`, `warn`, `info`, `debug`, `trace`) | `info` |

> **Default prompt**: `Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long.`

> **Backwards Compatibility**: The deprecated `IMMICH_ANALYZE_OLLAMA_HOSTS` variable is still supported and will be automatically mapped to `IMMICH_ANALYZE_HOSTS` when `IMMICH_ANALYZE_INTERFACE=ollama`.

### Command Line Arguments

```bash
Usage: immich-analyze [OPTIONS]

Options:
  -m, --monitor
          Enable folder monitoring mode
  -c, --combined
          Enable combined mode: process existing images then monitor for new ones
  -o, --overwrite-existing
          Overwrite existing entries in database (process all files regardless of existing descriptions)
      --immich-root <IMMICH_ROOT>
          Path to Immich root directory (containing upload/, thumbs/ folders) [default: /var/lib/immich]
      --postgres-url <POSTGRES_URL>
          PostgreSQL connection string (used only in database mode) [default: "host=localhost user=postgres dbname=immich password=your_password"]
  -d, --data-access-mode <DATA_ACCESS_MODE>
          Data access mode: database (direct PostgreSQL) or api (Immich REST API) [default: database] [possible values: database, immich-api]
      --immich-api-url <IMMICH_API_URL>
          Immich API base URL (required when using api access mode) [env: IMMICH_API_URL=]
      --immich-api-key <IMMICH_API_KEY>
          Immich API authentication key (required when using api access mode) [env: IMMICH_API_KEY]
      --api-poll-interval <API_POLL_INTERVAL>
          API poll interval in seconds (for Immich API mode) [default: 10]
      --model-name <MODEL_NAME>
          Ollama model name for image analysis [default: qwen3-vl:4b-thinking-q4_K_M]
      --interface <INTERFACE>
          AI service interface type [default: ollama] [possible values: ollama, llamacpp]
      --hosts <HOSTS>
          Host URLs (Ollama or llama.cpp server) [default: http://localhost:11434]
      --api-key <API_KEY>
          API key for authentication (llama.cpp server) [env: IMMICH_ANALYZE_API_KEY]
      --max-concurrent <MAX_CONCURRENT>
          Maximum number of concurrent requests [default: 4]
      --unavailable-duration <UNAVAILABLE_DURATION>
          Host availability check interval in seconds [default: 60]
      --timeout <TIMEOUT>
          HTTP request timeout in seconds [default: 300]
      --file-write-timeout <FILE_WRITE_TIMEOUT>
          File write timeout in seconds [default: 30]
      --file-check-interval <FILE_CHECK_INTERVAL>
          File stability check interval in milliseconds [default: 500]
      --event-cooldown <EVENT_COOLDOWN>
          Minimum time between processing identical events in seconds [default: 2]
      --prompt <PROMPT>
          Prompt for generating image description [default: "Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long."]
      --lang <LANG>
          Interface language (ru, en) [default: ""]
      --max-retries <MAX_RETRIES>
          Maximum number of retry attempts (0 = infinite) [default: 0]
      --retry-delay-seconds <RETRY_DELAY_SECONDS>
          Delay between retry cycles in seconds (fixed) [default: 5]
  -h, --help
          Print help (see more with '--help')
  -V, --version
          Print version
```

> **Note**: `IMMICH_API_URL` and `IMMICH_API_KEY` are read from environment variables by clap when using `--data-access-mode immich-api` - no need to pass them as command-line arguments.

## Usage Examples

### Database Mode

**Basic Batch Processing with Ollama**
```bash
immich-analyze \
  --data-access-mode database \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama-server:11434"
```

**Basic Batch Processing with llama.cpp Server**
```bash
immich-analyze \
  --data-access-mode database \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface llamacpp \
  --hosts "http://llamacpp-server:8080"
```

**Monitor Mode (Watch for new images)**
```bash
immich-analyze \
  --monitor \
  --data-access-mode database \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama:11434,http://ollama-backup:11434"
```

**Monitor Mode with Infinite Retries**
```bash
immich-analyze \
  --monitor \
  --data-access-mode database \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama:11434" \
  --retry-delay-seconds 10
```

**Batch Processing with Limited Retries**
```bash
immich-analyze \
  --data-access-mode database \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama-server:11434" \
  --max-retries 3 \
  --retry-delay-seconds 10
```

### API Mode

**Basic Batch Processing via Immich API**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --data-access-mode immich-api \
  --interface ollama \
  --hosts "http://ollama-server:11434"
```

**Combined Mode with API Access and llama.cpp**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
IMMICH_ANALYZE_API_KEY=your-llamacpp-api-key \
immich-analyze \
  --combined \
  --data-access-mode immich-api \
  --interface llamacpp \
  --hosts "http://llamacpp-primary:8080,http://llamacpp-secondary:8080" \
  --api-poll-interval 30
```

**Monitor Mode with Infinite Retries**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --data-access-mode immich-api \
  --interface ollama \
  --hosts "http://ollama:11434" \
  --monitor
```

**Batch Processing with Limited Retries**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --data-access-mode immich-api \
  --interface llamacpp \
  --hosts "http://llamacpp-server:8080" \
  --max-retries 5 \
  --retry-delay-seconds 15
```

### Enable Debug Logging
```bash
RUST_LOG=debug immich-analyze --combined --data-access-mode database --postgres-url "..." --interface ollama
```

## Model Recommendations

### For Ollama:
- `qwen3-vl:4b-thinking-q4_K_M` (Default) - Good balance of speed and accuracy
- `qwen3-vl:30b-a3b-thinking-q4_K_M` - Higher accuracy for complex images
- `qwen3-vl:2b-instruct-q4_K_M` - Faster processing for simpler descriptions

### For llama.cpp Server:
- Any GGUF vision model served via llama.cpp's OpenAI-compatible API
- Recommended: `qwen3-vl-4b-instruct-q4_k_m.gguf` or similar quantized variants

Install Ollama models using:
```bash
ollama pull qwen3-vl:4b-thinking-q4_K_M
```

## Architecture

The application integrates with your Immich instance by analyzing preview images and storing generated descriptions. It supports multiple operation modes:

- **Batch Mode**: Process all existing images in your library
- **Monitor Mode**: Automatically process new images as they're added to Immich
- **Combined Mode**: Process existing images in background while simultaneously monitoring for new additions

### Data Access Modes

#### Database Mode
- Direct access to Immich PostgreSQL database for reading/writing metadata
- Direct filesystem access to `thumbs/` directory for image analysis
- Uses filesystem events for monitoring new images
- Requires `--immich-root` and `--postgres-url` configuration

#### API Mode
- Uses Immich REST API for all data operations
- No direct database or filesystem access required
- Polls Immich API for new assets at configurable interval (`--api-poll-interval`)
- Requires `IMMICH_API_URL` and `IMMICH_API_KEY` environment variables

### Core Features
- Automatic retry logic with multiple AI service hosts and automatic failover
  - Configurable maximum retry attempts (`--max-retries`, 0 = infinite)
  - Configurable delay between retry cycles (`--retry-delay-seconds`)
  - Smart error classification: only retryable errors (5xx HTTP, timeouts, host unavailable) trigger retries
  - Non-retryable errors (invalid UUID, empty response, JSON parsing) fail immediately
- Host unavailability tracking with configurable recovery duration
- File stability checks (database mode) to ensure images are fully written before processing
- Event cooldown (database mode) to prevent duplicate processing of rapid filesystem events
- Structured logging via `env_logger` for easier debugging and monitoring

## Troubleshooting

### Enable verbose logging
Set the `RUST_LOG` environment variable to see detailed logs:
```bash
RUST_LOG=debug immich-analyze --combined ...
```

### Check AI service status
- For Ollama: `systemctl status ollama` or `curl http://localhost:11434/api/tags`
- For llama.cpp: `curl http://localhost:8080/health`

### API Mode Issues
- Verify `IMMICH_API_URL` is reachable: `curl $IMMICH_API_URL/api/server-info`
- Verify API key has sufficient permissions in Immich admin panel
- Check Immich server logs for authentication errors

## TODO:
- [x] Add llama.cpp support
- [x] Add support for Immich API
- [ ] ~~Add waiting list~~ Add retry logic
- [x] Rename ignore-existing option/variable to overwrite-existing
- [ ] Add JWT support
- [ ] Add NixOS service module
- [ ] Add video support
