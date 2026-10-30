# Super Simple Search Engine (SSSE)

A self-hosted, seed-driven search engine for Pages, Images, Videos and News.
One Rust binary serves the website, public JSON API, administration interface,
crawler and local administration socket. All persistent state lives in SQLite.
No external search service, frontend build pipeline or browser-rendering engine.

## Coverage and limits

The crawler discovers the wider web from your HTTPS seed URLs, including links
to new domains. Every crawl starts from seeds, never the old index. **This project
is DNS-limited:** it uses the operating system's resolver and can only discover
destinations allowed by the operator's DNS and network environment. SSSE does not
implement DNS filtering or guarantee that accessible content is safe or accurate.

- HTTPS-only ordinary crawling; IP-literal URLs and URL credentials are rejected.
- Mandatory robots compliance, including per-origin rules and website delays.
  Robots retrieval alone may follow HTTP/HTTPS redirects (at most five).
- Ordinary page redirects are recorded, not followed or used as replacement URLs.
- No JavaScript execution. JavaScript-only content may be unavailable.
- No PDFs, documents, archives, audio, downloadable videos or other file indexing.
  Image references and metadata are indexed; media binaries are not stored or proxied.
- Image previews, video thumbnails and favicons load directly from their sources.
  Those sites receive visitors' IP addresses. SSSE sends a `no-referrer` policy.
- Banned pages do not contribute outgoing links or popularity.
- Popularity is an incoming-link occurrence counter, **not PageRank**. Repeated
  eligible references count even when their crawl URLs are deduplicated.
- Promoted domains must match the query, but appear above ordinary matches.
- A successful crawl atomically replaces all published data. A stopped or failed
  crawl preserves the previous index. Interrupted staging is discarded at startup.

### Sitemaps, canonical hints, and refreshed restrictions

Sitemaps are discovered from robots declarations and HTML `link rel="sitemap"`
hints. If robots declares none, SSSE probes `/sitemap.xml` and `/sitemap_index.xml`
once per discovered origin. Namespace-aware URL sets, nested indexes, gzip files,
and HTTP gzip encoding are supported. The response-size limit bounds both received
and decompressed content; `0` leaves these limits disabled.

Depth is discovery distance, not the number of slashes in a URL. Sitemap metadata
retains its referring page's depth/domain through nested indexes, even on another
host. Within-domain page entries add one discovery step; external page entries
start at zero. Sitemap wrappers do not grant extra depth or reset it. Consequently,
a large sitemap can still offer many pages at the same depth; depth is not a
total-page limit. Sitemap documents are metadata, never searchable results.

Canonical hints are not redirects or permission overrides. SSSE considers
same-origin HTTPS hints, fetches their targets through ordinary eligibility/depth
checks, and consolidates only accepted pages with duplicate extracted text.
Missing, refused, different-content, cross-origin, and conflicting targets leave
the original page intact. Chains resolve to their accepted endpoint; cycles use
the lexicographically smallest cycle URL. News/video entries and image referring
URLs follow the representative during atomic publication. Canonical hints do not
add popularity points or override `nofollow`.

When robots rules refresh, newly prohibited pending tasks are removed from SQLite
and the FIFO, with one refusal per removed task. Leased workers remain accounted
for until they acknowledge completion and recheck restrictions before fetching.
Requests already sent cannot be retroactively unsent. Newly declared sitemaps are
discovered using the origin's existing depth context.

### Crawler settings

Settings, seeds, domain bans, word bans and promotions are stored in SQLite and
managed through Settings. Numeric crawler settings and bans are snapshotted at
crawl start; changes apply to the next crawl. Promotions affect searches immediately.

| Setting | Default | Meaning of 0 |
|---|---|---|
| Maximum domain depth | 10 | Unlimited |
| Maximum extracted text | 100 KiB | Unlimited |
| Maximum HTML/sitemap response | 1 MiB | Unlimited |
| Worker delay | 1,000 ms | No extra worker wait |
| Domain delay | 1,000 ms | No extra domain wait |
| Workers | Half available CPU threads, at least 1 | Automatic count |
| FIFO refill check | 1,000 ms | Continuous checks |
| Automatic crawl interval | 7 days | Manual crawling only |

FIFO capacity is always twice the effective worker count. Website delays remain
mandatory even when your own delays are zero. Effective domain spacing adds the
worker, domain and website intervals. Depth resets when entering a new registrable
domain. There is no arbitrary total page, domain or database-size cap.
Unrestricted depth/body/text settings can consume substantial memory and disk.
Network operations have connection and overall deadlines so a stalled server
cannot hold a worker forever.

The scheduler waits the configured interval after successful completion. Failed
or deliberately stopped attempts also postpone automatic retries by that interval.
This timing survives restarts. The admin status distinguishes running, stopped,
completed and failed crawls.

## Quick start with Nix

```sh
nix run github:MiPSR/super-simple-search-engine
```

Or from this checkout:

```sh
nix run .
```

The default listener is `127.0.0.1:3000`; persistent data defaults to `./ssse-data`.
Static files are embedded, so the binary works independently of your current directory.
For local **HTTP-only development**, explicitly allow non-Secure admin cookies:

```sh
SSSE_INSECURE_COOKIE=1 nix run . -- serve --data-dir ./ssse-data
```

In another terminal, initialize the password through the running instance:

```sh
nix run . -- password change --data-dir ./ssse-data
```

Use username `admin`, visit `/settings.html`, add HTTPS seeds and start a crawl.
There is no default password. Password input is hidden and confirmed. Password
changes invalidate all sessions. Browser theme, background and per-category page
size are public, browser-local preferences, independent of administrator access.

### CLI

```sh
ssse serve --data-dir /var/lib/ssse --bind 127.0.0.1:3000 --socket /run/ssse/ssse.sock
ssse password change --socket /run/ssse/ssse.sock
ssse crawl start --socket /run/ssse/ssse.sock
ssse crawl status --socket /run/ssse/ssse.sock
ssse crawl stop --socket /run/ssse/ssse.sock
ssse crawl restart --socket /run/ssse/ssse.sock
```

Environment equivalents: `SSSE_DATA_DIR`, `SSSE_BIND`, `SSSE_SOCKET`.
If no socket is specified, it lives at `<data-dir>/ssse.sock`. CLI commands talk
to the existing process; they never start another crawler or directly modify the DB.
Run them as the service user or root. The socket is mode `0600` in a protected directory.
One process may own a data directory at a time.

## NixOS service

Add an input and import its module:

```nix
{
  inputs.ssse.url = "github:MiPSR/super-simple-search-engine";
  outputs = { nixpkgs, ssse, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        ssse.nixosModules.default
        { services.ssse.enable = true; }
      ];
    };
  };
}
```

Options: `services.ssse.enable`, `package`, `bind`, `dataDir`, `insecureCookie`.
The module provides a restricted service user, persistent state, runtime socket,
restart policy and systemd hardening. Default storage is `/var/lib/ssse`.

```sh
sudo -u ssse /path/to/ssse password change --socket /run/ssse/ssse.sock
```

The package is also available as `inputs.ssse.packages.${pkgs.system}.default`.

## Complete NixOS container

Instead of the standalone service module, import the container module on the host:

```nix
{
  imports = [ inputs.ssse.nixosModules.container ];
  services.ssseContainer = {
    # Importing the module enables the container by default.
    bind = "127.0.0.1:3000";
    dataDir = "/var/lib/ssse-container";
  };
}
```

After `nixos-rebuild switch`, the `ssse` container auto-starts with the application,
service user, socket and persistent bind-mounted database directory. It shares the
host network, allowing the host's reverse proxy to reach the loopback listener.
For local HTTP testing only, set `services.ssseContainer.insecureCookie = true`.

```sh
sudo nixos-container run ssse -- /run/current-system/sw/bin/systemctl status ssse
sudo nixos-container run ssse -- /run/current-system/sw/bin/ssse password change --socket /run/ssse/ssse.sock
```

Container options: `enable`, `bind`, `dataDir`, `insecureCookie`. Do not enable the
standalone host service on the same listening address as the container.
`nix run` starts the portable application; NixOS container provisioning is supplied
by the container module, not by an unprivileged command changing your host system.

## Reverse proxy and operation

Production administration expects HTTPS and uses Secure, HttpOnly, SameSite cookies.
Place the service behind a TLS reverse proxy and keep the backend port private.
For example, standard Caddy provides TLS and proxying:

```caddyfile
search.example.org {
    reverse_proxy 127.0.0.1:3000
}
```

**That Caddy example does not implement rate limiting.** Stock Caddy does not ship
a general-purpose rate-limit directive. Add a maintained rate-limit module or put
a separate limiter in front of the backend. Configure distinct policies for search,
login attempts and authenticated mutations; also bound total connections/body sizes.
For an Nginx-based limiter, the relevant standard directives are, for example:

```nginx
# In http {}, then apply zones inside your existing TLS server.
limit_req_zone $binary_remote_addr zone=ssse_search:10m rate=5r/s;
limit_req_zone $binary_remote_addr zone=ssse_login:10m rate=5r/m;
limit_req_zone $binary_remote_addr zone=ssse_admin:10m rate=2r/s;

# Inside server {}:
location = /api/admin/login {
    limit_req zone=ssse_login burst=3 nodelay;
    proxy_set_header Host $host;
    proxy_pass http://127.0.0.1:3000;
}
location /api/admin/ {
    limit_req zone=ssse_admin burst=5 nodelay;
    proxy_set_header Host $host;
    proxy_pass http://127.0.0.1:3000;
}
location / {
    limit_req zone=ssse_search burst=15 nodelay;
    proxy_set_header Host $host;
    proxy_pass http://127.0.0.1:3000;
}
```

Tune these example limits for your community. Preserve the public Host header and
Origin/Referer headers so exact-origin CSRF validation works. Restrict direct origin
access, expose only necessary ports, protect database/backups/socket permissions,
and keep dependencies updated. SSSE does not implement an IP rate limiter or firewall.
SSSE's own UI serves noindex metadata, headers and restrictive robots.txt.

### Backups

Stop the service before copying `ssse.db`, or use SQLite's online backup API/CLI:

```sh
sqlite3 /var/lib/ssse/ssse.db ".backup '/safe/path/ssse-backup.db'"
```

Do not copy only a live WAL-mode database file and assume the backup is consistent.
Backups include credentials and sessions; restrict access. Restore while stopped.

## API

Public endpoints do not require authentication and may be used by third-party clients.
There is no unnecessary CORS policy.

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/search?q=hello&category=PAGES&page=1&per_page=10` | Search with pagination |
| GET | `/api/stats` | Latest successful snapshot and HTTP distribution |
| GET | `/api/stats/history` | Successful aggregate snapshots |
| GET | `/api/stats/top` | Top 10 indexed domains by popularity |
| GET | `/api/stats/bottom` | Bottom 10 indexed domains by popularity |

Categories are `PAGES`, `IMAGES`, `VIDEOS`, `NEWS`. Page sizes are 1–100;
defaults are 10/18/18/10. Search returns `{results,total,page,per_page}`.
Supported syntax: ordinary words (AND), `"quoted phrases"`, `-excluded`,
`site:example.com` (includes subdomains). Site-only and exclusion-only searches
are supported. Empty queries return no results. Malformed quotes are treated
predictably as literal phrase boundaries rather than raw FTS operators.
Ranking uses FTS5 BM25 with title weighting, a modest logarithmic popularity
boost and URL tie-breakers. Promotion is a separate first-order override.

Admin mutations require an authenticated `session` cookie and an exact matching
Origin/Referer, including login. For automation prefer the local CLI socket.

| Method | Path | JSON body / behavior |
|---|---|---|
| POST | `/api/admin/login` | `{ "username":"admin", "password":"..." }` |
| POST | `/api/admin/logout` | Revokes session and expires cookie |
| POST | `/api/admin/password` | `{ "username":"admin", "old_password":"...", "new_password":"..." }` |
| GET / PUT | `/api/admin/settings` | Read settings / merge validated numeric settings |
| GET / PUT | `/api/admin/seeds` | Array on GET; `{ "seeds":["https://example.org/"] }` on PUT |
| GET | `/api/admin/filters` | All bans and promotions |
| GET / PUT | `/api/admin/banned-domains` | Array on GET; `{ "items":["example.org"] }` on PUT |
| GET / PUT | `/api/admin/banned-words` | Array on GET; `{ "items":[{"word":"example","mode":"whole"}] }` on PUT |
| GET / PUT | `/api/admin/promoted` | Array on GET; `{ "items":["example.org"] }` on PUT |
| GET | `/api/admin/crawl/status` | State, counters, workers, FIFO/queue, last success and error |
| POST | `/api/admin/crawl/start` | Start; 409 if already active |
| POST | `/api/admin/crawl/stop` | Cancel and wait for cleanup |
| POST | `/api/admin/crawl/restart` | Stop and wait, then start from seeds |

Word modes: `whole` (Unicode alphanumeric boundaries) and `substring`, both
case-insensitive. Domain/seed updates validate the complete list before replacing it.
Server errors are logged without returning internal database details to clients.

## Development and portability

```sh
nix develop
cargo test
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
nix build
# Optional: real-browser smoke test (requires Chrome/Chromium on PATH).
cargo build
node tests/browser.mjs
# Full Nix checks, including a NixOS container VM test on Linux:
nix flake check
```

`nix-shell` also supplies Rust/Cargo, native libraries and Node for optional
JavaScript checks. Outside Nix, install stable Rust, a C compiler and development
tools; `cargo build --release` produces `target/release/ssse`. SQLite is bundled,
outbound TLS uses rustls with maintained certificate roots.

Tests include controlled local HTTPS fixtures; they do not crawl live websites.
No CI/CD service or GitHub Actions is required. Contributions should preserve the
small architecture and run the checks above. Chart.js is vendored locally under
its MIT license (`static/js/chart.LICENSE.txt`); no frontend asset requires a public CDN.
