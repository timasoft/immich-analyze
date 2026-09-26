# Immich Analyze

AI-powered image description generator for Immich photo management system

## Overview

Immich Analyze automatically generates detailed descriptions for images in your Immich library using AI vision models via **Ollama**, **llama.cpp server**, or **OpenRouter**. This enhances search capabilities and organization by providing semantic understanding of image content.

The application uses the Immich API (requires `IMMICH_API_URL` + `IMMICH_API_KEY`; supports multiple comma-separated keys for multi-user setups)

## Features

- AI-powered image analysis using Ollama, llama.cpp server, or OpenRouter with vision-capable models
- Multiple operation modes: batch processing, API monitoring, or combined mode
- Multi-host support with automatic failover for AI service endpoints
- Immich API integration
- Concurrent processing with configurable parallelism
- Configurable retry logic with max retries and delay between attempts
- Internationalization support (English and Russian)
- Docker container support
- Prompt enrichment: optionally enrich AI prompts with asset metadata (EXIF, location, camera info, people with ages, tags, resolution, MIME type)
- Selective description updates: use `--preserve-human` with any overwrite policy to preserve human-written text outside `[AI]...[/AI]` blocks; use `--overwrite-policy missing-ai` to process only assets without existing AI blocks
- Structured logging via `env_logger` (configure with `RUST_LOG` environment variable)
- Wait for Immich to become available on startup (configurable timeout)
- Startup model existence check against the configured AI hosts (skippable via `--no-preflight-model-check` / `IMMICH_ANALYZE_PREFLIGHT_MODEL_CHECK=false`). When enabled, any host that does not serve the configured model is blacklisted for the current run and will not be used until the app is restarted; a missing model on *all* hosts aborts startup

## Prerequisites

- Immich instance with an API endpoint and API key
- AI service running a vision-capable model:
  - **Ollama** server (e.g., `qwen3-vl:4b-thinking-q4_K_M`), OR
  - **llama.cpp server** with OpenAI-compatible API endpoint, OR
  - **OpenRouter** cloud API (OpenAI-compatible endpoint)

## Installation

### Docker Compose (Recommended)

To integrate Immich Analyze into your Immich setup, use the following `docker-compose.yaml` file:

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
      - /etc/localtime:/etc/localtime:ro
    env_file:
      - .env
    environment:
      # AI service configuration
      - IMMICH_ANALYZE_INTERFACE=ollama  # or "llamacpp" or "openrouter"
      - IMMICH_ANALYZE_HOSTS=http://ollama:11434
      # For llama.cpp server with authentication:
      # - IMMICH_ANALYZE_INTERFACE=llamacpp
      # - IMMICH_ANALYZE_HOSTS=http://llamacpp-server:8080
      # - IMMICH_ANALYZE_API_KEY=your-api-key-here
      # For OpenRouter (host defaults to https://openrouter.ai/api):
      # - IMMICH_ANALYZE_INTERFACE=openrouter
      # - IMMICH_ANALYZE_API_KEY=sk-or-xxx
      # Or use multiple hosts with automatic failover:
      # - IMMICH_ANALYZE_HOSTS=http://primary:11434,http://backup:11434
    depends_on:
      # Comment the next line if using external AI service
      - ollama
```

**Important notes about configuration:**

- You must provide API credentials (`IMMICH_API_URL`, `IMMICH_API_KEY`) for Immich API access
- The `ollama` service is **optional** - you can remove it and use an external Ollama, llama.cpp server, or OpenRouter instead
- Set `IMMICH_ANALYZE_INTERFACE` to `ollama` (default), `llamacpp`, or `openrouter` depending on your backend
- If using external service, modify `IMMICH_ANALYZE_HOSTS` to point to your server(s)
- For llama.cpp server, provide `IMMICH_ANALYZE_API_KEY` if authentication is enabled
- For OpenRouter, provide `IMMICH_ANALYZE_API_KEY` (starts with `sk-or-`); the host defaults to `https://openrouter.ai/api`
- After adding the Ollama service, you need to pull the model manually by executing:
  ```bash
  docker exec -it ollama ollama pull qwen3-vl:4b-thinking-q4_K_M
  ```
- For GPU acceleration with NVIDIA cards, uncomment the deploy section and ensure you have NVIDIA Container Toolkit installed

Make sure to:
1. Add the service(s) to your existing `docker-compose.yml` file
2. Add the required environment variables to your `.env` file

After adding the service, run:
```bash
docker-compose up -d immich-analyze
# If using internal Ollama service:
# docker-compose up -d ollama
```

### Nix

If you're using Nix or NixOS, you can build and run the application directly:

**API mode:**
```bash
IMMICH_API_URL=http://localhost:2283 IMMICH_API_KEY=your_key nix run github:timasoft/immich-analyze -- -c
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

   **API mode:**
   ```bash
   IMMICH_API_URL=http://localhost:2283 IMMICH_API_KEY=your_key immich-analyze -c
   ```

## Configuration

### Environment Variables (Docker)

#### Immich API Access Configuration

| Variable | Description | Default | Required For |
|----------|-------------|---------|-------------|
| `IMMICH_API_URL` | Immich API base URL | - | Required |
| `IMMICH_API_KEY` | Immich API authentication key(s) (comma-separated for multi-user setups) | - | Required |

#### AI Service Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `IMMICH_ANALYZE_INTERFACE` | AI service interface type (`ollama`, `llamacpp`, or `openrouter`) | `ollama` |
| `IMMICH_ANALYZE_HOSTS` | Comma-separated AI service host URLs | `http://localhost:11434` (`https://openrouter.ai/api` on `openrouter`) |
| `IMMICH_ANALYZE_API_KEY` | API key for llama.cpp server or OpenRouter authentication | *(none)* |
| `IMMICH_ANALYZE_MODEL_NAME` | Model name for image analysis | `qwen3-vl:4b-thinking-q4_K_M` |
| `IMMICH_ANALYZE_PROMPT` | Prompt for generating image descriptions | *See below* |
| `IMMICH_ANALYZE_ENRICH_PROMPT` | Enable prompt enrichment with asset metadata | `false` |
| `IMMICH_ANALYZE_API_POLL_INTERVAL` | API poll interval in seconds | `10` |

#### Application Settings

| Variable | Description | Default |
|----------|-------------|---------|
| `IMMICH_ANALYZE_MODE` | Operating mode: `monitor`, `combined`, or `batch` | `combined` |
| `IMMICH_ANALYZE_OVERWRITE_EXISTING` | If true, overwrite existing descriptions (alias for `--overwrite-policy all`) | `false` |
| `IMMICH_ANALYZE_OVERWRITE_POLICY` | Overwrite policy: `none` (skip any with description), `all` (process everything), `missing-ai` (process only if no `[AI]...[/AI]` block). Overrides `IMMICH_ANALYZE_OVERWRITE_EXISTING` | `none` |
| `IMMICH_ANALYZE_PRESERVE_HUMAN` | If true, preserve human text outside `[AI]...[/AI]` blocks by only replacing the AI block. Incompatible with `--disable-ai-wrapper` | `false` |
| `IMMICH_ANALYZE_LANG` | Interface language for the application (en, ru) | `en` |
| `IMMICH_ANALYZE_THUMBNAIL_SIZE` | Which Immich thumbnail rendition to analyze: `preview` (higher resolution, slower), `thumbnail` (lower resolution, faster) or `fullsize` (highest resolution, slowest) | `preview` |
| `IMMICH_ANALYZE_MAX_IMAGE_SIZE` | Downscale images whose longest edge exceeds this many pixels before sending them to the AI service (preserves aspect ratio). `0` disables downscaling | `0` |
| `IMMICH_ANALYZE_MAX_CONCURRENT` | Max concurrent AI requests | `4` |
| `IMMICH_ANALYZE_UNAVAILABLE_DURATION` | Host availability check interval in seconds | `60` |
| `IMMICH_ANALYZE_TIMEOUT` | AI request timeout in seconds | `300` |
| `IMMICH_ANALYZE_DISABLE_AI_WRAPPER` | If true, disable `[AI]...[/AI]` wrapper, storing description as plain text. Incompatible with `--preserve-human`. When combined with `missing-ai` overwrite policy, every asset will be re-analyzed (no `[AI]` tag to detect) | `false` |
| `IMMICH_ANALYZE_NO_FINAL_OUTPUT` | If true, disable final output with analysis results and statistics after batch processing | `false` |
| `IMMICH_ANALYZE_MAX_RETRIES` | Maximum retry attempts (0 = infinite) | `0` |
| `IMMICH_ANALYZE_RETRY_DELAY_SECONDS` | Delay between retry cycles in seconds | `5` |
| `IMMICH_ANALYZE_HEALTH_PORT` | Port for health check HTTP server (0 to disable) | `3000` |
| `IMMICH_ANALYZE_WAIT_FOR_IMMICH` | Wait for Immich to become available on startup | `true` |
| `IMMICH_ANALYZE_WAIT_TIMEOUT` | Maximum time in seconds to wait for Immich (0 = no limit) | `120` |
| `IMMICH_ANALYZE_WAIT_RETRY_INTERVAL` | Interval in seconds between retry attempts when waiting | `5` |
| `IMMICH_ANALYZE_PREFLIGHT_MODEL_CHECK` | Verify on startup that the configured model is served by at least one of the AI hosts | `true` |
| `RUST_LOG` | Logging level (`error`, `warn`, `info`, `debug`, `trace`) | `info` |

> **Default prompt**: `Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long.`

> **Backwards Compatibility**: The deprecated `IMMICH_ANALYZE_OLLAMA_HOSTS` variable is still supported and will be automatically mapped to `IMMICH_ANALYZE_HOSTS` when `IMMICH_ANALYZE_INTERFACE=ollama`.

### Command Line Arguments

```txt
Usage: immich-analyze [OPTIONS]

Options:
  -m, --monitor
          Enable API monitoring mode
  -c, --combined
          Enable combined mode: process existing images then monitor for new ones
  -o, --overwrite-existing
          Overwrite existing asset descriptions (process all assets regardless of existing descriptions) (same as --overwrite-policy all)
  -O, --overwrite-policy <OVERWRITE_POLICY>
          Overwrite policy [default: none]: none (skip any with description), all (process everything), missing-ai (process only if no [AI]...[/AI] block). Takes precedence over --overwrite-existing [possible values: none, all, missing-ai]
  -p, --preserve-human
          When overwriting or adding, preserve human-entered text by only replacing the [AI]...[/AI] block
      --immich-api-url <IMMICH_API_URL>
          Immich API base URL (required) [env: IMMICH_API_URL=]
      --immich-api-keys <IMMICH_API_KEYS>
          Immich API authentication key(s) (required). Provide multiple keys comma-separated for multi-user setups [env: IMMICH_API_KEY]
      --api-poll-interval <API_POLL_INTERVAL>
          API poll interval in seconds [default: 10]
      --thumbnail-size <THUMBNAIL_SIZE>
          Which Immich thumbnail rendition to analyze: preview (higher resolution, slower analysis), thumbnail (lower resolution, faster) or fullsize (highest resolution, slowest) [default: preview] [possible values: preview, thumbnail, fullsize]
      --max-image-size <MAX_IMAGE_SIZE>
          Downscale images whose longest edge exceeds this many pixels before sending them to the AI service (preserves aspect ratio). 0 disables downscaling [default: 0]
      --model-name <MODEL_NAME>
          Model name for image analysis [default: qwen3-vl:4b-thinking-q4_K_M]
      --interface <INTERFACE>
          AI service interface type [default: ollama] [possible values: ollama, llamacpp, openrouter]
      --hosts <HOSTS>
          Host URLs (Ollama, llama.cpp server, or OpenRouter) [default: http://localhost:11434]
      --api-key <API_KEY>
          API key for authentication (llama.cpp server or OpenRouter) [env: IMMICH_ANALYZE_API_KEY]
      --no-preflight-model-check
          Disable the startup model existence check against the configured AI hosts
      --max-concurrent <MAX_CONCURRENT>
          Maximum number of concurrent requests [default: 4]
      --unavailable-duration <UNAVAILABLE_DURATION>
          Host availability check interval in seconds [default: 60]
      --timeout <TIMEOUT>
          HTTP request timeout in seconds [default: 300]
      --prompt <PROMPT>
          Prompt for generating image description [default: "Create a detailed description for the image for proper image search functionality. In the response, provide only the description without introductory words. Also specify the image format (Wallpaper, Screenshot, Drawing, City photo, Selfie, etc.). The format must be correct. If in doubt, name the most likely option and don't think too long."]
      --lang <LANG>
          Interface language (ru, en) [default: ""]
      --max-retries <MAX_RETRIES>
          Maximum number of retry attempts (0 = infinite) [default: 0]
      --retry-delay-seconds <RETRY_DELAY_SECONDS>
          Delay between retry cycles in seconds (fixed) [default: 5]
      --enrich-prompt
          Enable prompt enrichment with asset metadata (date, location, camera info)
      --disable-ai-wrapper
          Disable [AI]...[/AI] wrapper around AI-generated description
      --no-final-output
          Disable final output with analysis results and statistics after batch processing
      --no-wait-for-immich
          Disable waiting for Immich to become available on startup
      --wait-timeout <WAIT_TIMEOUT>
          Maximum time in seconds to wait for Immich to become available (0 = no limit) [default: 120]
      --wait-retry-interval <WAIT_RETRY_INTERVAL>
          Interval in seconds between retry attempts when waiting for Immich [default: 5]
      --health-port <HEALTH_PORT>
          Port for health check HTTP server (0 to disable) [default: 3000]
  -h, --help
          Print help (see more with '--help')
  -V, --version
          Print version
```

> **Note**: `IMMICH_API_URL` and `IMMICH_API_KEY` are read from environment variables by clap - no need to pass them as command-line arguments. `IMMICH_API_KEY` supports multiple comma-separated keys for multi-user setups.

## Usage Examples

**Basic Batch Processing via Immich API**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --interface ollama \
  --hosts "http://ollama-server:11434"
```

**Multi-User Batch Processing (Multiple API Keys)**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=user1_key,user2_key,user3_key \
immich-analyze \
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
  --interface llamacpp \
  --hosts "http://llamacpp-primary:8080,http://llamacpp-secondary:8080" \
  --api-poll-interval 30
```

**Monitor Mode with Infinite Retries**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --interface ollama \
  --hosts "http://ollama:11434" \
  --monitor
```

**Batch Processing with Prompt Enrichment**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --interface ollama \
  --hosts "http://ollama-server:11434" \
  --enrich-prompt
```

**Batch Processing with Limited Retries**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --interface llamacpp \
  --hosts "http://llamacpp-server:8080" \
  --max-retries 5 \
  --retry-delay-seconds 15
```

**Batch Processing Without Final Results Output**
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
immich-analyze \
  --interface ollama \
  --hosts "http://ollama-server:11434" \
  --no-final-output
```

### Enable Debug Logging
```bash
IMMICH_API_URL=http://immich:2283 \
IMMICH_API_KEY=your_api_key \
RUST_LOG=debug \
immich-analyze \
  --combined \
  --interface ollama \
  --hosts "http://ollama-server:11434"
```

## Model Recommendations

### For Ollama:
- `qwen3-vl:4b-thinking-q4_K_M` (Default) - Good balance of speed and accuracy
- `qwen3-vl:30b-a3b-thinking-q4_K_M` - Higher accuracy for complex images
- `qwen3-vl:2b-instruct-q4_K_M` - Faster processing for simpler descriptions

### For llama.cpp Server:
- Any GGUF vision model served via llama.cpp's OpenAI-compatible API
- Recommended: `qwen3-vl-4b-instruct-q4_k_m.gguf` or similar quantized variants

### For OpenRouter:
- Any vision-capable model available on OpenRouter
- The host defaults to `https://openrouter.ai/api`; supply your API key via `IMMICH_ANALYZE_API_KEY` or `--api-key`

Install Ollama models using:
```bash
ollama pull qwen3-vl:4b-thinking-q4_K_M
```

## Architecture

The application integrates with your Immich instance by analyzing preview images and storing generated descriptions. It supports multiple operation modes:

- **Batch Mode**: Process all existing images in your library
- **Monitor Mode**: Automatically process new images as they're added to Immich
- **Combined Mode**: Process existing images in background while simultaneously monitoring for new additions

### Immich API
- Uses Immich REST API for all data operations
- No direct database or filesystem access required
- Polls Immich API for new assets at configurable interval (`--api-poll-interval`)
- Requires `IMMICH_API_URL` and `IMMICH_API_KEY` environment variables (supports multiple comma-separated keys)

#### Required API Key Permissions

The API key used must have the following permissions enabled in the Immich admin panel:

| Endpoint                     | Method | Permission                                      | Purpose                                     |
|------------------------------|--------|-------------------------------------------------|---------------------------------------------|
| `/api/search/metadata`       | POST   | `asset.read`                                    | List/search assets for batch processing     |
| `/api/assets/{id}/thumbnail` | GET    | `asset.view` (+ `asset.download` on `fullsize`) | Download thumbnail images for AI analysis   |
| `/api/assets/{id}`           | PUT    | `asset.update`                                  | Write generated descriptions back to assets |
| `/api/assets/{id}`           | GET    | `asset.read`                                    | Check asset existence and read metadata     |

> **Note**: Ensure all three permissions (`asset.read`, `asset.view`, `asset.update`) are enabled.

### Core Features
- Automatic retry logic with multiple AI service hosts and automatic failover
  - Configurable maximum retry attempts (`--max-retries`, 0 = infinite)
  - Configurable delay between retry cycles (`--retry-delay-seconds`)
  - Smart error classification: only retryable errors (5xx HTTP, timeouts, host unavailable) trigger retries
  - Non-retryable errors (invalid UUID, empty response, JSON parsing) fail immediately
  - Permanent provider rejections (e.g. a content-policy block returning `{"error":{"code":400,"message":"...PROHIBITED_CONTENT..."}}`) fail immediately, surface the provider's actual reason in the log, and are recorded as an `[AI] IMMICH-ANALYZE:BLOCKED: <reason> [/AI]` marker description so they are not re-attempted on subsequent runs
- Host unavailability tracking with configurable recovery duration
- Prompt enrichment: optionally enrich AI prompts with asset metadata (EXIF metadata, location, camera info, recognized people with ages, tags, resolution, MIME type) via the Immich API for more detailed descriptions
- Selective description preservation: when using `--preserve-human`, only the `[AI]...[/AI]` block in the description is replaced, preserving any human-written text outside this block. If no `[AI]...[/AI]` block exists, the AI-generated block is appended to the existing description
- Overwrite policies: use `--overwrite-policy all` to process everything, `--overwrite-policy none` to skip existing (default), or `--overwrite-policy missing-ai` to skip only assets with an existing `[AI]...[/AI]` block (processes human-only and empty descriptions)
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
- Verify `IMMICH_API_URL` is reachable: `curl $IMMICH_API_URL/api/server/ping`
- Verify API key has sufficient permissions in Immich admin panel
- Check Immich server logs for authentication errors

### Slow analysis on local vision backends

By default `immich-analyze` sends the Immich **preview** rendition to the AI service. If your `preview-size` is set high, local backends such as Ollama or `llama.cpp` have to encode large multimodal patches.

Use the thumbnail instead of the preview:
```bash
immich-analyze --thumbnail-size thumbnail ...
# or via env var in Docker:
# IMMICH_ANALYZE_THUMBNAIL_SIZE=thumbnail
```
Thumbnails are analyzed faster but at lower image quality, which may reduce the accuracy of the generated descriptions.

Downscale the image locally before sending it:
```bash
immich-analyze --max-image-size 1024 ...
# or via env var in Docker:
# IMMICH_ANALYZE_MAX_IMAGE_SIZE=1024
```

### Permanent provider rejections (content-policy / blocked content)

Remote providers sometimes permanently reject an image — for example returning `{"error":{"code":400,"message":"...PROHIBITED_CONTENT..."}}`. `immich-analyze` treats these as **non-retryable**: the request is not retried, and the asset is recorded as permanently blocked so later batch/monitor runs skip it instead of repeatedly paying for the same rejected request.

To re-attempt blocked assets, run with `--overwrite-policy all` (not recommended because it will reanalyze ALL photos and overwrite their descriptions), or clear the marker descriptions in Immich.

## TODO:
- [x] Add llama.cpp support
- [x] Add support for Immich API
- [x] ~~Add waiting list~~ Add retry logic
- [x] Rename ignore-existing option/variable to overwrite-existing
- [x] Add support for multiple Immich API keys
- [ ] Add JWT support
- [ ] Add NixOS service module
- [ ] Add video support
