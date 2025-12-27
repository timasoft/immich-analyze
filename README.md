# Immich Analyze

AI-powered image description generator for Immich photo management system

## Overview

Immich Analyze automatically generates detailed descriptions for images in your Immich library using Ollama's vision language models. This enhances search capabilities and organization by providing semantic understanding of image content.

## Features

- AI-powered image analysis using Ollama vision models
- Multiple operation modes: batch processing, folder monitoring, or combined mode
- Multi-host support with automatic failover for Ollama servers
- JWT authentication support for secure Ollama API access
- Direct integration with Immich PostgreSQL database
- Concurrent processing with configurable parallelism
- Internationalization support (English and Russian)
- Docker container support

## Prerequisites

- Immich instance with PostgreSQL database
- Ollama server running a vision-capable model (e.g., `qwen3-vl:4b-thinking-q4_K_M`)
- PostgreSQL database access for your Immich instance

## Installation

### Docker Compose Integration (Recommended)

To integrate Immich Analyze directly into your Immich setup, add the following service to your `docker-compose.yaml` file:

```yaml
services:
  # Optional: Ollama service (you can use external Ollama server instead)
  # This section is optional - remove it if you want to use external Ollama
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
      # Use internal Ollama service if defined above
      - IMMICH_ANALYZE_OLLAMA_HOSTS=http://ollama:11434
      # Or use external Ollama servers by uncommenting and modifying:
      # - IMMICH_ANALYZE_OLLAMA_HOSTS=http://external-ollama-server:11434,http://backup-ollama:11434
      # For Ollama servers requiring authentication:
      # - IMMICH_ANALYZE_OLLAMA_JWT_TOKEN=your_ollama_jwt_token_here
    depends_on:
      - database
      # Comment the next line if using external Ollama service
      - ollama
    networks:
      - immich-network

networks:
  immich-network:
    external: true
```

**Important notes about Ollama integration:**
- The `ollama` service is **optional** - you can remove it and use an external Ollama server instead
- If using external Ollama, modify `IMMICH_ANALYZE_OLLAMA_HOSTS` to point to your external server(s)
- After adding the service, you need to pull the model manually by executing:
  ```bash
  docker exec -it ollama ollama pull qwen3-vl:4b-thinking-q4_K_M
  ```
- For GPU acceleration with NVIDIA cards, uncomment the deploy section and ensure you have NVIDIA Container Toolkit installed
- If your Ollama instance requires authentication, provide a valid JWT token via `IMMICH_ANALYZE_OLLAMA_JWT_TOKEN` environment variable

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
| `IMMICH_ANALYZE_OLLAMA_HOSTS` | Comma-separated Ollama hosts | `http://localhost:11434` |
| `IMMICH_ANALYZE_OLLAMA_JWT_TOKEN` | JWT token for Ollama API authentication | `` (empty) |
| `IMMICH_ANALYZE_MODEL_NAME` | Ollama model to use | `qwen3-vl:4b-thinking-q4_K_M` |
| `IMMICH_ANALYZE_PROMPT` | Prompt for generating image descriptions | Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long. |
| `IMMICH_ANALYZE_IGNORE_EXISTING` | Ignore existing descriptions | `false` |
| `IMMICH_ANALYZE_LANG` | Interface language for the application (en, ru) | `en` |
| `IMMICH_ANALYZE_MAX_CONCURRENT` | Max concurrent requests | `4` |
| `IMMICH_ANALYZE_UNAVAILABLE_DURATION` | Ollama host availability check interval in seconds | `60` |
| `IMMICH_ANALYZE_TIMEOUT` | Request timeout in seconds | `300` |

### Command Line Arguments

```bash
Usage: immich-analyze [OPTIONS]

Options:
  -m, --monitor                          Enable folder monitoring mode
  -c, --combined                         Enable combined mode: process existing images and monitor for new ones
  -i, --ignore-existing                  Ignore existing entries in database
      --immich-root <IMMICH_ROOT>        Path to Immich root directory (containing upload/, thumbs/ folders) [default: /var/lib/immich]
      --postgres-url <POSTGRES_URL>      PostgreSQL connection string [default: host=localhost user=postgres dbname=immich password=your_password]
      --model-name <MODEL_NAME>          Ollama model name for image analysis [default: qwen3-vl:4b-thinking-q4_K_M]
      --ollama-hosts <OLLAMA_HOSTS>      Ollama host URLs [default: http://localhost:11434]
      --ollama-jwt-token <OLLAMA_JWT_TOKEN>
                                         JWT token for Ollama API authentication [default: ]
      --max-concurrent <MAX_CONCURRENT>  Maximum number of concurrent requests to Ollama [default: 4]
      --unavailable-duration <UNAVAILABLE_DURATION>
                                         Ollama host availability check interval in seconds [default: 60]
      --timeout <TIMEOUT>                HTTP/Ollama request timeout in seconds [default: 300]
      --file-write-timeout <FILE_WRITE_TIMEOUT>
                                         File write timeout in seconds [default: 30]
      --file-check-interval <FILE_CHECK_INTERVAL>
                                         File stability check interval in milliseconds [default: 500]
      --event-cooldown <EVENT_COOLDOWN>  Minimum time between processing identical events in seconds [default: 2]
      --prompt <PROMPT>                  Prompt for generating image description [default: Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long.]
      --lang <LANG>                      Interface language (ru, en) [default: ]
  -h, --help                             Print help
  -V, --version                          Print version
```

## Usage Examples

### Basic Batch Processing
```bash
immich-analyze \
  --postgres-url "host=localhost user=postgres dbname=immich password=password" \
  --ollama-hosts "http://secure-ollama-server:11434"
```

### Monitor Mode (Watch for new images)
```bash
immich-analyze \
  --monitor \
  --postgres-url "host=localhost user=postgres dbname=immich password=password"
```

### Combined Mode (Process existing + monitor new)
```bash
immich-analyze \
  --combined \
  --postgres-url "host=localhost user=postgres dbname=immich password=password"
```

## Model Recommendations

For optimal results, I recommend using these Ollama vision models:

- `qwen3-vl:4b-thinking-q4_K_M` (Default) - Good balance of speed and accuracy
- `qwen3-vl:30b-a3b-thinking-q4_K_M` - Higher accuracy for complex images
- `qwen3-vl:2b-instruct-q4_K_M` - Faster processing for simpler descriptions

Install models using:
```bash
ollama pull qwen3-vl:4b-thinking-q4_K_M
```

## Architecture

The application integrates with your Immich instance by analyzing preview images stored in the `thumbs/` directory and storing generated descriptions directly in the PostgreSQL database. It supports multiple operation modes:

- **Batch Mode**: Process all existing images in your library
- **Monitor Mode**: Automatically process new images as they're added to Immich
- **Combined Mode**: Process existing images in background while simultaneously monitoring for new additions

The system includes automatic retry logic with multiple Ollama hosts and handles file stability checks to ensure images are fully written before processing. When JWT authentication is enabled, the token is securely passed in the Authorization header using the Bearer scheme for all requests to Ollama API endpoints.

## TODO:
- [ ] Add waiting list
- [x] Add JWT support
- [ ] Add NixOS service module
- [ ] Add video support
