# Hosting

scry is one binary plus one SQLite file. It runs the same everywhere; pick
the host that fits and point every device's `[client] server_url` at it.

## Local and home-lab (laptop, mini-PC)

Bare metal is the simplest install:

```sh
cargo install --path crates/scry
cp deploy/scry.service ~/.config/systemd/user/
systemctl --user enable --now scry
loginctl enable-linger $USER    # start at boot, no login needed
```

Secrets (`TAVILY_API_KEY`, `SCRY_TOKEN`) live in `~/.config/scry/env`
(chmod 600), loaded by the unit's `EnvironmentFile` and referenced from
`config.toml` as `"env:VAR"`. The config file is then safe to track in
dotfiles; the env file never is.

The default config expects an OpenAI-compatible embedding endpoint on
`localhost:12434` (llama-swap, llama.cpp, Ollama). On a box without one,
use the compose bundle instead, which ships its own:

```sh
cd deploy
cp config.example.toml config.toml   # edit: embedding base_url = http://llama:8080/v1
SCRY_TOKEN=$(openssl rand -hex 24) docker compose up -d
```

### Reaching a home box from other devices

- Same LAN: set `listen = "0.0.0.0:7345"`, an `auth_token`, and use
  `http://<box>:7345` as the client `server_url`.
- Away from home: put the box on a WireGuard or Tailscale network and use
  its overlay address. Nothing about scry changes; it is just a TCP
  service. This is the recommended setup for a mini-PC.
- Public HTTPS: forward a port (or use a VPS as a TCP relay), run Caddy
  with `deploy/Caddyfile.example` for TLS, keep the bearer token on.

## Cloud / VPS and on-prem

Same compose bundle. CPU embedding of a 0.6B model is fast enough for
queries and incremental syncs; a large first index of a big repo is a
one-time cost. Two levers if that bothers you: run the first `scry index`
from a machine with a GPU-backed embedding endpoint, or run the whole
stack at home and move the single `.db` file later; the index carries no
host-specific state.

## Backups and migration

The entire index and memory store is one file (`db_path`). Copy it, and a
new host has everything. The embedding model and dimension are stamped
inside; a server configured with a different model refuses the file
instead of silently mixing vector spaces.
