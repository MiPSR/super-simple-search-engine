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
