# Immich Analyze

AI-powered image description generator for Immich photo management system

## Overview

Immich Analyze automatically generates detailed descriptions for images in your Immich library using AI vision models via **Ollama** or **llama.cpp server**. This enhances search capabilities and organization by providing semantic understanding of image content.

## Features

- AI-powered image analysis using Ollama or llama.cpp server with vision-capable models
- Multiple operation modes: batch processing, folder monitoring, or combined mode
- Multi-host support with automatic failover for AI service endpoints
- Direct integration with Immich PostgreSQL database
- Concurrent processing with configurable parallelism
- Internationalization support (English and Russian)
- Docker container support
- Structured logging via `env_logger` (configure with `RUST_LOG` environment variable)

## Prerequisites

- Immich instance with PostgreSQL database
- AI service running a vision-capable model:
  - **Ollama** server (e.g., `qwen3-vl:4b-thinking-q4_K_M`), OR
  - **llama.cpp server** with OpenAI-compatible API endpoint
- PostgreSQL database access for your Immich instance

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

**Important notes about AI service integration:**
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
```bash
nix run github:timasoft/immich-analyze --immich-root /path/to/immich/data --postgres-url "host=localhost user=your_postgres_user dbname=immich password=your_postgres_password" -c
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
   ```bash
   immich-analyze --immich-root /path/to/immich/data --postgres-url "host=localhost user=your_postgres_user dbname=immich password=your_postgres_password" -c
   ```

## Configuration

### Environment Variables (Docker)

| Variable | Description | Default |
|----------|-------------|---------|
| `DB_USERNAME` | PostgreSQL username | Required |
| `DB_PASSWORD` | PostgreSQL password | Required |
| `DB_DATABASE_NAME` | PostgreSQL database name | Required |
| `DB_HOSTNAME` | PostgreSQL hostname | `database` |
| `DB_PORT` | PostgreSQL port | `5432` |
| `IMMICH_ANALYZE_INTERFACE` | AI service interface type (`ollama` or `llamacpp`) | `ollama` |
| `IMMICH_ANALYZE_HOSTS` | Comma-separated AI service host URLs | `http://localhost:11434` |
| `IMMICH_ANALYZE_API_KEY` | API key for llama.cpp server authentication | *(none)* |
| `IMMICH_ANALYZE_MODEL_NAME` | Model name for image analysis | `qwen3-vl:4b-thinking-q4_K_M` |
| `IMMICH_ANALYZE_PROMPT` | Prompt for generating image descriptions | Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long. |
| `IMMICH_ANALYZE_IGNORE_EXISTING` | If true, the program will overwrite existing descriptions | `false` |
| `IMMICH_ANALYZE_LANG` | Interface language for the application (en, ru) | `en` |
| `IMMICH_ANALYZE_MAX_CONCURRENT` | Max concurrent requests | `4` |
| `IMMICH_ANALYZE_UNAVAILABLE_DURATION` | Host availability check interval in seconds | `60` |
| `IMMICH_ANALYZE_TIMEOUT` | Request timeout in seconds | `300` |
| `RUST_LOG` | Logging level for debugging (`error`, `warn`, `info`, `debug`, `trace`) | `info` |

> **Backwards Compatibility**: The deprecated `IMMICH_ANALYZE_OLLAMA_HOSTS` variable is still supported and will be automatically mapped to `IMMICH_ANALYZE_HOSTS` when `IMMICH_ANALYZE_INTERFACE=ollama`.

### Command Line Arguments

```bash
Usage: immich-analyze [OPTIONS]

Options:
  -m, --monitor
          Enable folder monitoring mode
  -c, --combined
          Enable combined mode: process existing images then monitor for new ones
  -i, --ignore-existing
          Ignore existing entries in database
      --immich-root <IMMICH_ROOT>
          Path to Immich root directory (containing upload/, thumbs/ folders) [default: /var/lib/immich]
      --postgres-url <POSTGRES_URL>
          PostgreSQL connection string [default: "host=localhost user=postgres dbname=immich password=your_password"]
      --model-name <MODEL_NAME>
          Ollama model name for image analysis [default: qwen3-vl:4b-thinking-q4_K_M]
      --interface <INTERFACE>
          AI service interface type [default: ollama] [possible values: ollama, llamacpp]
      --hosts <HOSTS>
          Host URLs (Ollama or llama.cpp server) [default: http://localhost:11434]
      --api-key <API_KEY>
          API key for authentication (llama.cpp server)
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
          Interface language (ru, en) [default: ]
  -h, --help
          Print help
  -V, --version
          Print version
```

## Usage Examples

### Basic Batch Processing with Ollama
```bash
immich-analyze \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama-server:11434"
```

### Basic Batch Processing with llama.cpp Server
```bash
immich-analyze \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface llamacpp \
  --hosts "http://llamacpp-server:8080" \
```

### Monitor Mode (Watch for new images)
```bash
immich-analyze \
  --monitor \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface ollama \
  --hosts "http://ollama:11434,http://ollama-backup:11434"
```

### Combined Mode (Process existing + monitor new)
```bash
immich-analyze \
  --combined \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --interface llamacpp \
  --hosts "http://llamacpp-primary:8080,http://llamacpp-secondary:8080" \
  --api-key "your-api-key"
```

### Enable Debug Logging
```bash
RUST_LOG=debug immich-analyze --combined --postgres-url "..." --interface ollama
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

The application integrates with your Immich instance by analyzing preview images stored in the `thumbs/` directory and storing generated descriptions directly in the PostgreSQL database. It supports multiple operation modes:

- **Batch Mode**: Process all existing images in your library
- **Monitor Mode**: Automatically process new images as they're added to Immich using filesystem events
- **Combined Mode**: Process existing images in background while simultaneously monitoring for new additions

The system includes:
- Automatic retry logic with multiple AI service hosts and automatic failover
- Host unavailability tracking with configurable recovery duration
- File stability checks to ensure images are fully written before processing
- Event cooldown to prevent duplicate processing of rapid filesystem events
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

## TODO:
- [x] Add llama.cpp support
- [ ] Add support for Immich API
- [ ] Add waiting list
- [ ] Rename ignore-existing option/variable to overwrite-existing
- [ ] Add JWT support
- [ ] Add NixOS service module
- [ ] Add video support
