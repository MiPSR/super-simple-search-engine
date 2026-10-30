# Super Simple Search Engine (SSSE)

A lightweight self-hosted search engine written in Rust.

## Build & Run

Using Nix:
```
nix develop
cargo build
nix run .
```

Outside Nix:
```
cargo build --release
./target/release/ssse serve --data-dir /var/lib/ssse --bind 127.0.0.1:3000
```

## Initialize password
```
ssse password change   # prompts for password
```

## Configure seeds
Use the Settings page (admin) or API.

## Crawl
```
ssse crawl start
ssse crawl status
ssse crawl stop
```

## Reverse proxy
Place behind Caddy for TLS, rate limiting, etc. SSSE itself does not implement HTTP rate limiting.

## Notes
- Robots compliance is mandatory and cannot be disabled.
- HTTPS only for normal pages.
- JavaScript is not executed during crawling.
## Reverse proxy (Caddy example)
```
example.com {
    reverse_proxy 127.0.0.1:3000
}
```
Rate limiting is handled externally (Caddy modules or separate services). SSSE does not implement IP rate limiting.

## Backup
Copy the SQLite database while the service is stopped, or use SQLite's backup API.

## Persistent storage
Set `--data-dir` to a writable persistent directory. The DB is created there.

## robots.txt
SSSE serves its own `robots.txt` with noindex to avoid external indexing of the SSSE UI.

## Installation (Nix)
```
nix develop
nix build
nix run
```

## NixOS module
Add flake input and enable `services.ssse.enable = true;`.

## Container
Run the built binary in a NixOS container with a persistent data directory.

## Password management
```
ssse password change
```

## Crawler control
```
ssse crawl start
ssse crawl stop
ssse crawl restart
ssse crawl status
```

## Search API
`GET /api/search?q=...&category=PAGES&per_page=10&page=1`
Categories: PAGES, IMAGES, VIDEOS, NEWS.

## Robots compliance
Mandatory; cannot be disabled. HTTPS-only normal crawling. JavaScript is not executed.

## Crawling and indexing
Seeds define starting points. Each crawl rebuilds the index from seeds only; previous index retained until success.

## Domain popularity
Incoming link occurrence counter (not PageRank). Repeated references count.

## JavaScript-free crawling
No JS execution; JS-only content may be missing.

## Excluded URLs and bans
Banned domains/words applied at crawl time; take effect next crawl.

## Using the GitHub flake
`nix run github:owner/ssse`

## NixOS input
Add as flake input and import the module.

## Enable module
`services.ssse.enable = true;`

## Container
Use `nixos-container` or any OCI runtime with persistent data dir.

## nix-shell
`nix-shell` provides Rust toolchain.

## Build outside Nix
`cargo build --release`

## Seed URLs
Configure in Settings (admin) or API.

## Public API
See `/api/search`, `/api/stats`, etc.

## Contributing
PRs welcome; run `cargo test` and `cargo fmt`.

## Architecture
Single Rust binary: HTTP server, REST API, crawler, search, admin UI, CLI control socket. One SQLite DB.

## Dependencies
Rust stable, Tokio, Axum, rusqlite, SQLite FTS5, reqwest, scraper, argon2, psl.

## Crawling behavior
Concurrent workers, per-domain cooldowns, robots compliance mandatory, HTTPS only, no JS execution.

## Index replacement
Staging index swapped atomically on success; previous index retained on failure.

## Limitations
JS-only content unavailable; no PDF/document indexing; no image binaries stored.
