<!--
SPDX-License-Identifier: MIT OR Apache-2.0
Copyright 2026 coccinella-labs
-->

# Harper Docker Guide



This guide provides detailed instructions for running Harper using Docker, including setup, configuration, and troubleshooting.

## Requirements

- **Docker**: Latest stable version ([install Docker](https://docs.docker.com/get-docker/))
- **Docker Compose**: For advanced setups (included with Docker Desktop)
- **Git**: For cloning the repository
- **Operating System**: Linux, macOS, or Windows with Docker support

## Installation

### Quick Start

Harper provides pre-built Docker images for easy deployment.

1. **Clone the repository**:
   ```bash
   git clone https://github.com/coccinella-labs/harper.git
   cd harper
   ```

2. **Configure environment**:
   ```bash
   cp config/env.example .env
   # Edit .env with your API keys (see Configuration section below)
   ```

3. **Build and run**:
   ```bash
   docker build -t harper .
   docker run --rm -it --env-file .env -v harper_data:/app/data harper
   ```

### Detailed Setup

#### Building the Image

```bash
# Standard build
docker build -t harper .

# Build with custom tag
docker build -t my-harper:latest .

# Build with no cache (force rebuild)
docker build --no-cache -t harper .
```

#### Running the Container

##### Interactive Mode (Recommended)
```bash
# With environment variables
docker run --rm -it --env-file .env harper

# Without env vars (limited functionality)
docker run --rm -it harper
```

##### Background Mode
```bash
# Run in background
docker run -d --name harper-container --env-file .env harper

# View logs
docker logs harper-container

# Stop container
docker stop harper-container
```

#### Using Docker Compose

Docker Compose provides persistent data storage and easier management.

```bash
# Start with compose
docker-compose up --build

# Run in background
docker-compose up -d --build

# View logs
docker-compose logs

# Stop
docker-compose down
```

#### Data Persistence

Harper stores sessions in a SQLite database. With Docker Compose, data persists in a named volume.

To persist data with plain Docker:
```bash
# Mount host directory (Unix/Linux/macOS)
docker run --rm -it -v $(pwd)/data:/app/data --env-file .env harper

# Windows Command Prompt
docker run --rm -it -v %cd%/data:/app/data --env-file .env harper

# Windows PowerShell
docker run --rm -it -v ${pwd}/data:/app/data --env-file .env harper
```

## Sandbox Isolation

Harper runs with sandbox isolation in Docker when `exec_policy.sandbox.enabled = true`.

### Enable Sandbox

Use `config/docker.toml` which has sandbox enabled by default:

```bash
docker run --rm -it -v $(pwd)/config:/app/config --env-file .env harper
```

Or manually set in your config:

```toml
[exec_policy.sandbox]
enabled = true
allowed_dirs = ["/app"]
network_access = false
readonly_home = true
max_execution_time_secs = 30
```

### How It Works

- **bubblewrap**: Linux namespace isolation
- Restricts filesystem access to allowed directories only
- Disables network by default (set `network_access = true` to enable)
- Timeouts commands after 30 seconds

## Configuration

### Environment Variables

Harper uses environment variables for configuration. Set one of the following API keys in your `.env` file:

```bash
# Choose your AI provider
export OPENAI_API_KEY="your-openai-key"
# OR
export SAMBASTUDIO_API_KEY="your-sambanova-key"
# OR
export GEMINI_API_KEY="your-gemini-key"

# Optional: Custom database location
export DATABASE_PATH="./harper.db"
```

### Custom Configuration

Mount custom config files:
```bash
docker run --rm -it -v $(pwd)/my-config:/app/config --env-file .env harper
```

## Troubleshooting

### Common Issues

1. **GLIBC errors**: Ensure you're using a compatible base image (fixed in current Dockerfile).

2. **Config not found**: The Dockerfile includes config files; ensure the image is up-to-date.

3. **Menu looping**: Use `-it` flag for interactive mode.

4. **API key errors**: Verify your .env file has correct keys.

### Debugging

```bash
# Check container logs
docker logs <container-id>

# Run with verbose output
docker run --rm -it --env-file .env -e RUST_LOG=debug harper

# Inspect running container
docker exec -it <container-id> /bin/bash
```

## Advanced Usage

### Multi-Stage Builds

The Dockerfile uses multi-stage builds for optimized image size.

### CI/CD

Docker builds are automatically tested and published in GitHub Actions on pushes to main/develop branches.

## Examples

### Basic Chat Session
```bash
docker run --rm -it --env-file .env harper
# Select option 1 for new chat session
```

### List Sessions
```bash
docker run --rm -it --env-file .env harper
# Select option 2 to list sessions
```

### Export History
```bash
docker run --rm -it --env-file .env harper
# Select option 4 to export session history
```

## Support

For issues with Docker setup, check:
- Docker and Docker Compose versions
- Environment variables in .env
- Network connectivity for API calls
- Sufficient disk space for builds

For additional help, see our [Contributing Guide](../CONTRIBUTING.md) or [GitHub Issues](https://github.com/coccinella-labs/harper/issues).
