# Super Simple Search Engine (SSSE)

## 1. Project Overview

Super Simple Search Engine (SSSE) is a lightweight, self-hosted, open-source search engine written in Rust, with a native HTML, CSS, and JavaScript frontend.

Its purpose is to crawl websites, discover links, index content, calculate domain popularity, and provide a simple search interface.

The project prioritizes simplicity, maintainability, modern technologies, and minimal dependencies.

### 1.1. Core Principles

- One Rust application providing the HTTP server, REST API, crawler, search engine, and administration interface.
- One SQLite database containing all persistent application data, organized into separate tables.
- Rust backend using stable Rust and established, actively maintained libraries.
- Vanilla HTML, CSS, and JavaScript frontend, without a frontend framework or Node.js toolchain.
- No Redis, Elasticsearch, PostgreSQL, external worker services, or other application infrastructure.
- No headless browsers or JavaScript execution during crawling.
- No binary media downloads or storage.
- Mandatory compliance with website crawling restrictions.
- No artificial limit on the total number of indexed websites or pages.
- Rebuild the search index from configured seed URLs on every crawl.
- Preserve the previous successful index until a new crawl completes successfully.
- Nix-first development and deployment, while remaining portable to other Linux environments.
- Use current stable technologies, not nightly, unstable, or experimental dependencies.
- Keep HTTP abuse prevention and request rate limiting external to the application.

The project must remain understandable, relatively small, and practical to maintain.

### 1.2. Application Name

**Super Simple Search Engine**

Abbreviation: **SSSE**

The frontend displays the application name using individually colored letters.

---

## 2. Technology Stack

### 2.1. Backend

Required technologies:

| Component | Technology |
|---|---|
| Language | Rust, latest stable toolchain |
| Async runtime | Tokio |
| HTTP server | Axum |
| Database | SQLite |
| SQLite integration | rusqlite |
| Full-text search | SQLite FTS5 |
| Search relevance | BM25 |
| HTTP client | reqwest |
| HTML parsing | Established Rust HTML parsing libraries |
| Serialization | serde / serde_json |
| Password hashing | Argon2id |
| Logging | tracing |
| Domain identification | Maintained Public Suffix List implementation |

Equivalent, well-maintained libraries may be selected where appropriate.

Prefer Rust's standard library when it already provides the required functionality.

Avoid introducing unnecessary dependencies.

### 2.2. Frontend

- HTML5.
- Modern CSS.
- Vanilla JavaScript using ES modules.
- CSS custom properties for styling.
- OKLCH for colors.
- Native browser APIs wherever practical.
- Chart.js, or an equivalently established lightweight library, for statistics graphs.
- localStorage for small browser preferences.
- IndexedDB for custom background image storage.

No React, Vue, Angular, Svelte, or similar frameworks.

No frontend compilation or bundling step should be necessary.

Third-party frontend assets must be supplied locally rather than requiring a public CDN.

### 2.3. HTTP Architecture

The Rust application serves:

- Static frontend resources.
- Public read-only API endpoints.
- Authenticated administration API endpoints.
- Application-specific `robots.txt`.

The application should be designed to operate behind an external reverse proxy.

HTTP/3 and public-facing TLS termination are the responsibility of the reverse proxy, not the Rust application.

---

## 3. Project Structure and Nix Integration

### 3.1. Repository

The repository must contain:

- Rust source code.
- Frontend static files.
- Database schema and migration code.
- `Cargo.toml`.
- `Cargo.lock`.
- `flake.nix`.
- `flake.lock`.
- `shell.nix`.
- NixOS module.
- Appropriate `.gitignore`.
- README documentation.
- Automated tests.

A simple, logical directory structure is preferred.

Do not split the project into unnecessary workspaces or microservices.

### 3.2. Flake Requirements

The flake must provide:

- A default package.
- A default runnable application.
- A development shell.
- A reusable NixOS module.

The application must support commands such as:

- `nix run github:<owner>/<repository>`
- `nix build github:<owner>/<repository>`
- `nix develop`

It must also be usable as a package through another Nix flake input.

A traditional `shell.nix` must be provided for development using `nix-shell`.

The development environment should provide the Rust compiler, Cargo, Rust formatting and linting tools, and any necessary native libraries.

### 3.3. Build Policy

- Never commit compiled executables.
- Compile from source locally or through Nix.
- No GitHub Actions.
- No CI/CD service requirement.
- No Docker requirement.
- Pin reproducible dependency versions using lockfiles.
- Update dependencies to maintained stable releases.
- Avoid experimental dependencies and nightly compiler features.

### 3.4. Deployment

The executable must support running in a container, particularly a NixOS container.

The design must not depend on NixOS-specific runtime functionality.

Support normal Linux installations and distributions.

The application database and other persistent runtime state must reside in a writable persistent directory.

The NixOS module should provide reasonable options for enabling the service, selecting the package, configuring the listening interface and port, and specifying the persistent data directory.

The service must have appropriate filesystem permissions and systemd integration.

No TOML or other application configuration file is required.

All persistent application settings belong in SQLite.

Deployment-specific options such as listening addresses and database paths may be supplied by the process environment, command-line startup arguments, or the service manager.

---

## 4. Database Architecture

### 4.1. General Requirements

Use one SQLite database file containing multiple tables.

Use SQLite WAL mode to support search queries while the crawler writes data.

Use transactions, prepared statements, constraints, and indexes where appropriate.

Avoid holding database write transactions open during HTTP requests, parsing, or worker delays.

The database design must support:

- Persistent application settings.
- Authentication.
- Seed URLs.
- Banned domains.
- Banned words.
- Promoted domains.
- Published search results.
- Search indexes.
- Domain popularity.
- Refused URLs and refusal reasons.
- HTTP response information.
- Robots rules.
- Temporary crawl data.
- Historical aggregate statistics.

The exact implementation may use additional tables and indexes where necessary.

### 4.2. Data Categories

There are three major categories of database data.

**Persistent application data**

Settings, authentication, seed URLs, ban lists, promotion lists, and historical aggregate statistics.

This data survives every crawl.

**Published crawl data**

The latest successfully completed crawl's pages, images, videos, news, domain popularity, refused URLs, HTTP results, and related metadata.

This data is replaced only when a new crawl succeeds.

**Temporary crawl data**

The active crawl's queue, discovered URLs, domain dictionary, robots information, intermediate records, and staging indexes.

This data is independent of the previous published index and is deleted after completion, cancellation, or failure.

The database must use a schema that clearly separates these categories.

### 4.3. Suggested Tables

Persistent tables:

- `settings`
- `credentials`
- `sessions`
- `seed_urls`
- `banned_domains`
- `banned_words`
- `promoted_domains`
- `crawl_history`

Published index tables:

- `pages`
- `images`
- `videos`
- `news`
- `domains`
- `domain_popularity`
- `refused_urls`
- `http_responses`
- `robots`
- Appropriate FTS5 virtual tables.

Temporary crawl tables:

- `crawl_queue`
- `crawl_seen_urls`
- `crawl_domains`
- `crawl_robots`
- `crawl_responses`
- `crawl_refused_urls`
- `crawl_links`
- Staging versions of the published index tables.

These names are recommendations, not a restriction on the schema.

Temporary crawl tables must not become permanent historical stores.

### 4.4. HTTP Status Storage

Record HTTP response status codes when available, including successful and unsuccessful requests.

Examples include:

- 200
- 301
- 302
- 403
- 404
- 429
- 500
- 503

Also support non-HTTP failures, such as DNS resolution failures, using a separate error reason rather than inventing an HTTP status code.

Keep the relevant HTTP results from the latest successful crawl.

Do not preserve a full historical record of every individual HTTP request across multiple completed crawls.

### 4.5. SQLite Concurrency

SQLite must not unnecessarily block the frontend while crawling.

Requirements:

- WAL mode.
- Short write transactions.
- Coordinated database writes.
- Separate read access for the web server.
- Appropriate indexes.
- Managed checkpointing.
- No large database operations on asynchronous runtime threads that could block the HTTP server.

rusqlite operations are synchronous, so blocking operations must be isolated appropriately.

---

## 5. Crawling Architecture

### 5.1. General Behavior

Crawling is concurrent and multi-threaded.

Multiple workers process discovered URLs while respecting configured intervals and mandatory website restrictions.

The crawler must not interfere significantly with normal search requests.

There is only one possible active crawl.

### 5.2. Seed URLs

A configurable list of HTTPS URLs defines the starting points.

At the beginning of every crawl:

1. Create fresh temporary crawl structures.
2. Read the configured seed URLs.
3. Insert them into the new crawl queue.
4. Initialize the worker pool.
5. Begin discovering and processing links.

**It is strictly forbidden to use URLs from the previous published index as starting points or additional crawl inputs.**

The new crawl must discover everything independently.

The seed list persists between crawls.

Seed URLs must be validated and deduplicated.

### 5.3. URL Discovery

For each eligible HTML page:

1. Fetch its content.
2. Validate the response and crawling permissions.
3. Extract relevant content and metadata.
4. Check content against banned terms.
5. If accepted, index the content.
6. Extract permitted hyperlinks.
7. Update destination-domain popularity counters for references.
8. Insert eligible newly discovered URLs into the current crawl queue.

A page containing banned content must not contribute indexed content or discovered links.

Repeated references to the same destination may still contribute additional popularity points.

The crawl queue must deduplicate URLs to prevent fetching the same URL repeatedly within one crawl cycle.

URL deduplication must not suppress popularity counts.

### 5.4. Workers

Default worker count:

`max(1, floor(available CPU threads / 2))`

Worker count is configurable.

Use Tokio asynchronous tasks to execute crawling operations concurrently across the runtime's threads.

Each worker is responsible for:

- Consuming a task from the FIFO.
- Respecting its own configurable request interval.
- Checking the shared domain cooldown.
- Checking robots requirements.
- Reserving the next permitted request time before requesting a URL.
- Downloading eligible HTML within configured limits.
- Parsing and validating content.
- Producing database updates.
- Discovering and enqueueing eligible links.

Worker scheduling must not require complex domain-aware FIFO coordination.

### 5.5. FIFO Queue

The crawler uses:

- A persistent SQLite crawl queue.
- A bounded in-memory FIFO queue.

The FIFO capacity is always:

`worker_count * 2`

This value is derived and **must not be exposed as a separate setting**.

Default queue-refill check interval: **1 second**.

Whenever the pending in-memory FIFO contains fewer tasks than the active worker count, refill it from the SQLite queue until it reaches capacity or no pending tasks remain.

Tasks assigned to workers must be tracked so they cannot be assigned twice.

Persistent queue rows should be marked as assigned or leased rather than prematurely discarded.

A worker must acknowledge completion or rejection.

Workers may encounter several tasks from the same domain.

The coordinator must not attempt to reorder the FIFO by domain cooldown.

### 5.6. Rate Limiting

Three types of intervals exist:

1. Per-worker request interval.
2. Per-registrable-domain request interval.
3. Website-requested crawling interval.

Defaults:

| Interval | Default |
|---|---|
| Worker interval | 1 second |
| Domain interval | 1 second |
| Website interval | Derived from website rules |

All applicable intervals must be respected.

The worker interval applies independently to each worker.

For a given domain, the effective domain reservation interval includes the configured worker interval, the domain interval, and any additional website-requested delay.

For example:

- Worker interval: 1 second.
- Domain interval: 1 second.
- Website crawl delay: 5 seconds.

The effective minimum interval between requests to that registrable domain is 7 seconds.

The worker's independent cooldown must also be respected.

Use a thread-safe in-memory dictionary keyed by registrable domain to store the next permitted request time.

The worker checks the domain timer and atomically reserves the next request slot **before starting the request**.

The timer operation must prevent two workers from simultaneously bypassing the domain delay.

Workers may wait asynchronously.

Use monotonic time for in-memory cooldown calculations.

Do not implement special retry or backoff behavior for ordinary HTTP 429 or 503 page responses.

Record their HTTP statuses and continue processing other tasks.

Robots.txt retrieval errors are handled separately according to robots requirements.

### 5.7. Domain Identification

Use registrable domains, also called eTLD+1, determined through a maintained Public Suffix List implementation.

Examples:

- `en.wikipedia.org` belongs to `wikipedia.org`.
- `www.wikipedia.org` belongs to `wikipedia.org`.
- `example.co.uk` is distinct from `other.co.uk`.

The shared domain cooldown and popularity score use this registrable domain.

Robots rules are tracked separately for each origin or hostname as required by the robots protocol.

Subdomains may have different robots rules even when they share the same registrable-domain cooldown.

### 5.8. Crawl Depth

Default maximum crawl depth per registrable domain: **10**.

Configurable through the settings page.

Depth starts at zero when the crawler enters a domain through a seed or an external link.

Following internal links increases the depth.

External entry points must not inherit an unrelated domain's accumulated internal depth.

Once the maximum permitted depth is reached, do not enqueue deeper internal links.

### 5.9. Resource Limits

| Setting | Default |
|---|---|
| Maximum extracted text per HTML page | 100 KiB |
| Maximum downloaded HTML response body | 1 MiB |
| Maximum crawl depth per domain | 10 |

These settings are configurable.

Pages exceeding the configured size limits are rejected and recorded with an appropriate refusal reason.

Response bodies must be read with a strict limit so oversized pages do not have to be downloaded fully before rejection.

No artificial total index-size or domain-count limit should be introduced.

### 5.10. Crawl Scheduling

Default automatic crawl interval: **7 days**.

The interval is measured from completion of the previous successful crawl.

Only one crawl can be active.

If a scheduled crawl attempt occurs while another crawl is active, ignore it.

Do not queue overlapping crawl instances.

A failed or manually stopped crawl must not cause an uncontrolled immediate automatic retry loop.

The scheduler must maintain consistent timing across application restarts.

### 5.11. Crawl Completion

A crawl is complete when:

- All reachable, eligible queued URLs have been processed or rejected.
- No worker is processing a task.
- No eligible queued tasks remain.
- Final indexes and aggregate measurements have been successfully prepared.

Ordinary page failures do not make the whole crawl fail.

Unrecoverable internal failures, such as database corruption or fatal indexing errors, must prevent publication.

The application must never publish a partially completed crawl as a successful result.

### 5.12. Stopping and Restarting

Stopping a crawl must:

1. Prevent new tasks from being assigned.
2. Cancel active crawling operations safely.
3. Wait for or cancel outstanding database work.
4. Remove all temporary crawl data.
5. Preserve the last successful published index.

Restarting a crawl must stop the current crawl, discard its temporary data, and begin again exclusively from the configured seed URLs.

A program crash during crawling must never corrupt the previously published index.

On application startup, incomplete staging data from an interrupted crawl must be identified and discarded.

---

## 6. Robots and Website Restrictions

### 6.1. Mandatory Compliance

Compliance with website crawling restrictions is mandatory.

There must be **no setting, CLI flag, or API endpoint capable of disabling robots compliance**.

Implement applicable behavior from RFC 9309.

Also recognize established crawling directives such as:

- `robots.txt` Allow and Disallow rules.
- Crawl-delay extensions.
- Applicable request-rate directives when supported.
- Sitemap declarations.
- HTML `noindex`.
- HTML `nofollow`.
- Link-level `nofollow`.
- HTTP `X-Robots-Tag` directives.

Only apply directives relevant to crawling or indexing.

### 6.2. Robots Retrieval

Robots files are retrieved separately from ordinary crawl pages.

They are not indexed or treated as search results.

Rules are associated with their original website origin.

Initially retrieve robots.txt once per origin during a crawl and cache the result.

Store:

- Original robots URL.
- Applicable origin.
- Retrieval timestamp.
- HTTP status.
- Parsed rules.
- Crawl delay and other relevant directives.
- Cache validity or refresh information.
- Retrieval errors when applicable.

Refresh cached rules after 24 hours when a crawl remains active long enough, subject to the protocol's caching and unreachable-server rules.

### 6.3. Robots Redirects

Allow up to five consecutive redirects when fetching robots.txt.

This applies only to robots retrieval.

The discovered rules continue to apply to the original website origin, not the redirected URL's domain.

Robots retrieval may follow HTTP or HTTPS redirects where appropriate.

IP-literal destinations remain ineligible.

If retrieval fails or exceeds the redirect limit, apply conservative protocol-compliant behavior.

### 6.4. Robots Failures

Follow RFC 9309's distinction between unavailable and unreachable robots files.

A missing robots file returning an ordinary 4xx response may allow crawling, as permitted by the protocol.

An unreachable robots file caused by server or network failures must prevent crawling of the affected origin while the restriction applies.

Do not apply ordinary webpage failure behavior to robots retrieval.

### 6.5. Disallowed Content

When robots rules prohibit crawling a URL:

- Do not fetch its page content.
- Do not index it.
- Do not follow links from it.
- Record the refused URL and reason.
- Remove matching pending tasks from the SQLite queue and in-memory FIFO when applicable.

A newly discovered restriction must be applied to already queued tasks.

Pending or active workers must check current restrictions before beginning a request.

### 6.6. Sitemaps

Discover sitemaps through robots.txt and other standard methods where available.

Sitemap URLs are candidate crawl inputs, not exceptions to crawling rules.

Every sitemap-derived URL must undergo the same HTTPS, domain, ban-list, duplicate, depth, and robots checks as an ordinary discovered URL.

Sitemaps must not override seed-based crawl initialization.

---

## 7. URL and File Handling

### 7.1. HTTPS Requirement

Only HTTPS URLs may be crawled and indexed as normal search content.

Reject:

- HTTP pages.
- IP-literal URLs.
- Unsupported URL schemes.
- URLs that trigger file downloads.
- URLs that violate robots rules.
- URLs excluded by administrator-defined content restrictions.

The HTTPS-only restriction does not apply identically to the special robots.txt retrieval process.

### 7.2. DNS

Use the operating system's configured DNS resolver.

Do not embed a separate DNS service or content-category filtering engine.

Do not hardcode blocked website categories.

DNS and network filtering are managed by the deployment environment.

Direct IP-address URLs must be rejected, including IPv4 and IPv6 literals.

Normal crawler redirects are not followed, so redirect targets are not added as crawl replacements.

The README must explain that the crawler operates within the destinations permitted by the configured DNS and network environment, subject to its HTTPS, robots, and administrator-defined restrictions.

### 7.3. Redirects

For ordinary page requests:

- Do not automatically follow HTTP redirects.
- Record the response status.
- Do not replace the requested URL with the redirect destination.
- Do not add redirect destinations to the queue merely because of a redirect.

This does not affect the separate robots.txt redirect procedure.

### 7.4. File Downloads

Non-image files must not enter the search index.

Examples:

- PDF files.
- Archives.
- Executables.
- Documents.
- Audio files.
- Direct video files.
- Other downloadable binary formats.

A link to such a file may remain part of the source page's readable text, but the file must not be crawled or indexed separately.

Use URL information, response headers, and MIME types to identify resources.

Do not rely exclusively on file extensions because URLs may not contain them.

Any response indicating an attachment download must be rejected.

Do not download binary file bodies merely to classify them.

### 7.5. Images

Direct image resources are the exception to the general file exclusion rule.

Index their URLs and available metadata without storing or downloading full image content.

Use metadata from the referring HTML page and lightweight HTTP header checks where necessary.

Do not proxy or cache remote images.

The frontend loads images directly from the source URLs.

---

## 8. Content Extraction

### 8.1. HTML Pages

Extract relevant information from eligible HTML, including:

- Canonical URL where applicable.
- Page title.
- Meta description.
- Readable text.
- Relevant headings.
- Language metadata where available.
- Article publication date where available.
- Links.
- Image references and alt text.
- Video references and metadata.
- News/article metadata.
- Robots directives.
- Relevant structured data.

Normalize extracted text for indexing.

Do not store unnecessary HTML markup.

Avoid indexing navigation boilerplate and duplicated interface text where practical.

### 8.2. JavaScript Websites

Do not execute JavaScript while crawling.

Do not bundle Chromium, Firefox, WebKit, or another headless browser engine.

Extract information available directly through HTTP responses, HTML, metadata, and structured data.

JavaScript-only content may be unavailable to the crawler.

This limitation is explicitly accepted.

### 8.3. Publication Dates

Use a publication date only when the page or article provides one.

Valid sources include appropriate article metadata and structured data.

Do not substitute the crawl date for a missing publication date.

Do not invent publication dates.

Store crawl and last-check timestamps separately for internal tracking.

---

## 9. Search Categories

The application has exactly four search categories:

1. **PAGES**
2. **IMAGES**
3. **VIDEOS**
4. **NEWS**

These categories are accessible through tabs in the search results header.

### 9.1. Pages

Pages represent indexed website pages.

Each result displays:

- Favicon, loaded directly from its source.
- Title.
- URL.
- Publication date, only if available.

No locally stored favicon files.

A page must satisfy all applicable content and robots restrictions.

### 9.2. Images

Image results represent direct image URLs discovered during crawling.

Store appropriate metadata such as:

- Image URL.
- Referring page.
- Title or descriptive context.
- Alt text.
- Source website.
- Publication date, if available.

Display image results in a grid.

Desktop default:

- 18 results per page.
- 6 images per row.
- 3 rows per page.

The layout must adapt responsively to smaller screens.

Each result displays:

- Image preview loaded directly from the image URL.
- Favicon loaded directly from the source website.
- Source URL.
- Title.
- Publication date, if available.
- Alt text, if available.

Clicking an image opens a larger preview and provides access to its original URL.

Never store image binaries in SQLite.

### 9.3. Videos

Videos represent video pages and embedded video players from video services.

Examples include dedicated video pages and embedded YouTube players.

Do not index direct downloadable video files as videos.

Extract available:

- Video title.
- Video page or player URL.
- Source website.
- Thumbnail URL.
- Publication date, if available.
- Relevant descriptive metadata.

Recognize video pages through HTML metadata, structured data, and known embedding patterns.

Do not execute video players during crawling.

Do not download videos.

Display videos using a thumbnail-based grid similar to image results.

Each result displays:

- Thumbnail, loaded directly.
- Favicon, loaded directly.
- URL.
- Title.
- Publication date, if available.

A result should link to the original video page or player.

### 9.4. News

News represents web pages identified as articles or news content through available article metadata and structured data.

News search uses the same basic visual result format as Pages:

- Favicon.
- URL.
- Title.
- Publication date, if available.

Do not classify all recently crawled pages as news.

Publication date does not determine whether a page belongs in News.

The crawler must use available article-identification signals.

---

## 10. Search Indexing and Ranking

### 10.1. Full-Text Index

Use SQLite FTS5.

Use built-in BM25 ranking for textual relevance.

Index appropriate searchable fields, including:

- Titles.
- Descriptions.
- Readable page text.
- Relevant image alt text.
- Relevant video metadata.
- Article metadata.

Different search categories may use separate FTS5 indexes if that simplifies queries and maintenance.

### 10.2. BM25

BM25 is responsible for determining textual relevance.

Prioritize relevance over domain popularity.

More relevant results should generally appear before less relevant results.

Domain popularity may improve ordering among similarly relevant results.

Use supported FTS5 field weighting where useful, with titles weighted more heavily than ordinary body text.

Do not implement a separate custom TF-IDF engine.

### 10.3. Domain Popularity

Popularity is a simple incoming-link occurrence counter.

It is **not PageRank**.

For every accepted link reference discovered during crawling, increment the destination registrable domain's popularity counter.

Example:

If 50 pages on Wikipedia reference `example.com`, those references contribute 50 popularity points to `example.com`.

Repeated references count individually.

Do not collapse references by referring website, referring domain, or destination page for popularity scoring.

URL deduplication for the crawl queue remains independent of popularity counting.

All pages belonging to the same registrable domain share that domain's popularity score.

No recursive reputation calculations, trust heuristics, content-quality scoring, or complex link weighting.

### 10.4. Promoted Domains

Administrators can configure promoted registrable domains.

Promoted domains:

- Still must match the search query.
- Bypass normal popularity-based ordering.
- Appear above otherwise eligible ordinary results.
- Remain subject to bans and mandatory robots restrictions.
- Do not create results for pages that do not match the query.

Promotion is a separate override, not an alteration of the stored popularity score.

### 10.5. Search Operators

Support at least:

- Ordinary multiword searches.
- Quoted phrases.
- Negative terms using `-term`.
- Domain filtering using `site:example.com`.

Translate search input safely into FTS5-compatible queries.

Do not concatenate untrusted search input into SQL.

Use predictable handling of malformed operators and empty queries.

### 10.6. Pagination

Use numbered pagination.

No infinite scrolling.

Result counts are browser preferences, independently configurable for each category.

Defaults:

| Category | Results per page |
|---|---|
| Pages | 10 |
| Images | 18 |
| Videos | 18 |
| News | 10 |

Result pagination must be stable and deterministic for a given published index and query.

---

## 11. Administrator Content Controls

### 11.1. Banned Domains

Administrators may maintain a list of banned domains.

Banned domains must not appear in search results.

Do not crawl explicitly banned domains.

Apply the ban to applicable subdomains.

Store relevant refusals with the reason `banned_domain` or an equivalent identifier.

Do not add banned URLs to the searchable index.

### 11.2. Banned Words

Administrators may maintain a list of banned words or text patterns.

Each entry supports a matching mode:

- Whole-word matching.
- Substring matching.

Matching must be case-insensitive and Unicode-aware.

Banned terms are checked against:

- URL.
- Domain name.
- Page title.
- Extracted page text.
- Relevant indexed metadata.

For example, a banned substring appearing in a domain or page content causes the page to be rejected.

If a banned term matches:

- Reject the page.
- Do not index its content.
- Do not follow links from it.
- Do not count outgoing links from that rejected page toward popularity.
- Record only the relevant refusal metadata and reason.

Rejected content must not remain in searchable tables.

### 11.3. Ban-List Timing

Banned domain and word lists are applied during a crawl.

Changes to these lists do not dynamically rewrite or filter the current published index.

They take effect on the next crawl.

This behavior must be clear in the settings interface.

### 11.4. Refused URLs

Maintain a dedicated refused-URLs table for the latest successful crawl.

Possible refusal reasons include:

- Banned domain.
- Banned word.
- Robots restriction.
- Unsupported protocol.
- HTTP URL.
- IP-literal URL.
- Unsupported file type.
- Attachment download.
- Oversized page.
- Maximum depth.
- `noindex`.
- HTTP error.
- DNS or connection failure.
- Invalid URL.
- Other explicit refusal reason.

Record relevant metadata such as:

- URL.
- Hostname or domain.
- Reason.
- HTTP status, if available.
- Timestamp.

Do not store the rejected page's readable content.

Refused URLs are not exposed in public search results.

The refused-URLs table is replaced on successful crawls along with the published index.

---

## 12. Crawling Transactions and Index Replacement

### 12.1. Active and Staging Indexes

Maintain a published index and a separate staging index inside the SQLite database.

During crawling:

- All new crawl data goes into temporary/staging tables.
- The published index remains available.
- Search requests continue using the published index.
- No staging results are exposed publicly.

### 12.2. Successful Crawl

After a crawl completes successfully:

1. Finish all staging writes.
2. Complete FTS5 index preparation.
3. Validate consistency of the new index and related tables.
4. Calculate final aggregate statistics.
5. Atomically activate the new published data.
6. Remove the previous published index and its associated records.
7. Delete remaining temporary crawl tables and queue data.
8. Store the successful crawl's historical statistics snapshot.
9. Update the latest successful crawl timestamp and duration.

The previous index must be completely removed after successful replacement.

No old page-index generations should be retained.

The transition must not expose a mixture of old and new search results.

### 12.3. Stopped or Failed Crawl

If a crawl is stopped, restarted, or suffers a fatal failure:

- Discard its temporary tables.
- Do not replace the published index.
- Do not update the latest successful crawl snapshot.
- Preserve the previously published data.

Individual HTTP errors and rejected pages are normal crawling outcomes, not fatal crawler failures.

---

## 13. Statistics

### 13.1. Statistics Page

The Stats page is public.

No authentication is required.

It displays information from the latest successful published crawl and historical aggregate snapshots.

### 13.2. Current Statistics

Include useful measurements such as:

- Total indexed pages.
- Total indexed websites/domains.
- Total indexed images.
- Total indexed videos.
- Total indexed news entries.
- Total refused URLs.
- Total processed URLs.
- HTTP response status distribution.
- Latest crawl duration.
- Latest successful crawl date.
- Average page retrieval time.
- Relevant crawling and indexing totals.

The exact collection of additional measurements may be expanded where data is already available.

Do not introduce unnecessary external telemetry services.

### 13.3. Historical Statistics

After each successful crawl, save one aggregate snapshot.

The snapshot should contain:

- Completion timestamp.
- Crawl duration.
- Total domains.
- Total pages.
- Total images.
- Total videos.
- Total news.
- Total refused URLs.
- Total processed requests.
- Selected HTTP status counts.
- Other useful numeric aggregates.

Do not store historical copies of every indexed page.

Historical statistics are intended to show the evolution of the search engine between completed crawls.

### 13.4. Graphs

Use interactive graphs.

Requirements:

- Multiple numeric data series.
- Independently toggleable series.
- Ability to show or hide measurements on the same graph.
- Clear legends.
- Appropriate time-based X axis.
- Readable units.
- Responsive layout.
- Accessible labels and alternative numeric information.

Include crawl duration so unexpectedly long or short crawl cycles are easy to identify.

### 13.5. Popularity Rankings

Display:

- Top 10 highest-popularity domains.
- Top 10 lowest-popularity indexed domains.

Popularity means the number of recorded incoming link references.

Display the domain and its numerical popularity value.

A domain with zero references may appear in the lowest-popularity ranking.

---

## 14. Frontend Design

### 14.1. General Appearance

The interface should resemble a modern, minimal search engine.

Requirements:

- Clean design.
- Responsive layout.
- No unnecessary animations.
- No heavy frontend frameworks.
- Minimal resource usage.
- CSS custom properties for all reusable design values.
- OKLCH-based color definitions.
- WCAG 2.2 AA accessibility target for functional UI components.

Use semantic HTML and native browser behavior.

### 14.2. Homepage

The homepage has:

**Top-left:**

Application name, displayed with randomly colored letters.

**Top-right:**

- Settings link using a settings emoji.
- Stats link using a statistics emoji.

**Main area:**

- Large `SEARCH` heading.
- Each letter receives its own random color.
- Search bar centered horizontally, slightly below the viewport's vertical center.
- Search button.

**Footer:**

A link to the project's GitHub repository.

### 14.3. Random Letter Colors

Each letter receives a random hue.

Color format:

`oklch(0.5 0.05 H)`

Where `H` is a randomly generated hue between 0 and 360 degrees.

Use the native browser API:

`crypto.getRandomValues()`

Do not add a randomness library.

Generate new colors on every page refresh.

Apply a thin outline using the normal interface text-color CSS variable, **not the letter's own randomized color**.

The outline must respond to theme changes.

For example, the design may use a CSS custom property such as `--text-color` for the outline.

The colored branding is decorative. Functional controls, labels, and ordinary text must meet accessibility contrast requirements.

### 14.4. Search Results Layout

After a search:

- The large central `SEARCH` heading disappears.
- The search bar moves into the top header.
- The application name remains on the left.
- The search bar and button appear beside it.
- Settings and Stats remain accessible on the right.
- The four category tabs appear in the top section, above the results.
- Search results appear below the header.

The header must remain usable on smaller screens.

### 14.5. Category Tabs

Display:

- PAGES
- IMAGES
- VIDEOS
- NEWS

The active category must be visually distinguishable.

Changing categories should retain the current search query.

Each category maintains its own pagination behavior.

### 14.6. Result Layout

Pages and News use a conventional vertical list.

Images and Videos use responsive thumbnail grids.

External favicons, image previews, and thumbnails are loaded directly from their sources.

No server-side image proxy is required.

Provide sensible visual fallbacks for missing or unavailable thumbnails and favicons.

### 14.7. Accessibility

Implement:

- Semantic HTML.
- Keyboard navigation.
- Visible focus indicators.
- Accessible names for emoji-only links and icon buttons.
- Appropriate contrast for ordinary text and controls.
- Accessible form validation.
- Logical heading structure.
- Responsive layout.
- Reduced-motion support.
- Appropriate ARIA attributes where native HTML is insufficient.
- Accessible graph legends and numerical alternatives.

Avoid unnecessary ARIA when native semantic elements already provide the required behavior.

---

## 15. Browser Preferences

Browser preferences must not be stored in the server database.

They are local to each browser.

### 15.1. Theme

Supported themes:

- Follow System.
- Light.
- Dark.

Default: **Follow System**.

Store the theme selection in localStorage.

Use the native `prefers-color-scheme` media query for system theme detection.

Apply the theme without unnecessary page flashes.

### 15.2. Custom Background

Allow the user to select a local image as the website background.

Store the image using IndexedDB.

Store lightweight activation or preference metadata in localStorage as appropriate.

Requirements:

- Background is active when an image is configured.
- Display a preview in Settings.
- Show an `X` remove button above the preview.
- Removing the background restores the normal theme background.
- The background resizes automatically with the viewport.
- No JavaScript-based cropping or positioning system.

Use native CSS:

`background-size: cover`

`background-position: center`

`background-repeat: no-repeat`

Cropping caused naturally by `cover` is acceptable.

Do not stretch or distort the image.

The custom background applies across the application.

Maintain functional UI readability over the background.

### 15.3. Results Per Page

Allow the user to choose results per page independently for:

- Pages.
- Images.
- Videos.
- News.

Store the values in localStorage.

Defaults:

- Pages: 10.
- Images: 18.
- Videos: 18.
- News: 10.

Validate stored values and restore defaults when browser data is invalid.

---

## 16. Settings Page

### 16.1. Authentication

The Settings page is protected by a password.

The user must authenticate before modifying server settings.

Use:

- Argon2id for password hashing.
- Server-side sessions.
- Secure, HttpOnly, SameSite session cookies.
- Appropriate CSRF protection for state-changing requests.
- Session expiration.
- Session invalidation after password changes.

Do not store plaintext passwords.

Do not store administrative session credentials in localStorage.

No default hardcoded password.

Provide a secure initial-password setup process through the local administration CLI.

### 16.2. Server Settings

All server settings are stored in SQLite.

The interface must expose the following settings:

| Setting | Default |
|---|---|
| Automatic crawl interval | 7 days |
| Number of crawl workers | Half CPU threads, minimum 1 |
| Per-worker delay | 1 second |
| Per-domain delay | 1 second |
| FIFO refill check interval | 1 second |
| Maximum depth per domain | 10 |
| Maximum extracted text size | 100 KiB |
| Maximum HTML response body | 1 MiB |
| Seed URLs | Empty list |
| Banned domains | Empty list |
| Banned words | Empty list |
| Promoted domains | Empty list |

FIFO capacity is always twice the worker count and is not independently configurable.

Robots compliance is mandatory and must not be configurable as an on/off setting.

### 16.3. Settings Behavior

Provide clear validation and appropriate units.

Settings must persist across service restarts.

Changes affecting crawler structure or worker counts must be applied safely.

Changes that require a new crawl should be identified in the interface.

Settings must not silently corrupt or partially rewrite the published index.

### 16.4. Password Management

Provide a password-change form.

Require appropriate verification of the existing password for changes initiated through the web interface.

Hash the new password using Argon2id.

Invalidate existing administrative sessions after a successful password change.

### 16.5. Crawl Management

The authenticated settings page may provide administrative controls to:

- Start crawling.
- Stop crawling.
- Restart crawling.
- Inspect current crawl status.

These actions use the same internal application control logic as the CLI.

Display enough feedback to distinguish a running, stopped, completed, and failed crawl.

### 16.6. Browser Settings

The browser-preferences section is separate from server settings.

It contains:

- Theme selection.
- Background image upload, preview, and removal.
- Results-per-page preferences.

Browser preferences remain client-side.

---

## 17. Command-Line Interface

### 17.1. General Architecture

The CLI must communicate with the existing application process.

It must not start a separate crawler or web server.

Use a local Unix domain socket for administrative communication.

The socket must have restrictive filesystem permissions.

The CLI sends commands to the running application, which validates and executes them.

The application is responsible for changing the database and coordinating workers safely.

### 17.2. Required Commands

The executable must provide commands equivalent to:

| Command | Behavior |
|---|---|
| `password change` | Change or initialize the administrative password |
| `crawl start` | Start a crawl if none is active |
| `crawl stop` | Stop and discard the current crawl |
| `crawl restart` | Stop the current crawl and begin a fresh one |
| `crawl status` | Display current crawler and worker information |

Use a consistent executable name such as `ssse`.

CLI password input must be secure and must not expose passwords in shell history or ordinary process arguments.

### 17.3. Crawl Status

The status command must report useful information such as:

- Current crawl state.
- Start time.
- Elapsed time.
- Configured worker count.
- Active workers.
- Waiting or idle workers.
- FIFO task count.
- Persistent queued task count.
- Number of processed URLs.
- Number of refused URLs.
- Last successful crawl.
- Latest successful crawl duration.
- Relevant errors, if any.

It must not expose administrative credentials or session secrets.

---

## 18. API

### 18.1. General Requirements

The frontend is an API client.

The Rust server exposes the necessary JSON API.

Use conventional REST-style routes and HTTP methods.

The API must be documented in the repository.

Public read-only endpoints are intentionally accessible to third-party clients.

Do not attempt to enforce a browser-only API.

### 18.2. Suggested Public Endpoints

| Endpoint | Purpose |
|---|---|
| `GET /api/search` | Search indexed content |
| `GET /api/stats` | Current published statistics |
| `GET /api/stats/history` | Historical aggregate measurements |
| `GET /api/stats/top` | Highest-popularity domains |
| `GET /api/stats/bottom` | Lowest-popularity domains |

Search requests must support:

- Query.
- Category.
- Page number.
- Results per page.

Responses should return result data and pagination metadata.

### 18.3. Suggested Administration Endpoints

| Endpoint | Purpose |
|---|---|
| `POST /api/admin/login` | Authenticate |
| `POST /api/admin/logout` | End session |
| `POST /api/admin/password` | Change password |
| `GET /api/admin/settings` | Read server settings |
| `PUT /api/admin/settings` | Update server settings |
| `GET /api/admin/seeds` | Read seed URLs |
| `GET /api/admin/filters` | Read bans and promotions |
| `GET /api/admin/crawl/status` | Read detailed crawl status |
| `POST /api/admin/crawl/start` | Start crawl |
| `POST /api/admin/crawl/stop` | Stop crawl |
| `POST /api/admin/crawl/restart` | Restart crawl |

Appropriate authenticated mutation endpoints must also exist for seed URLs, banned words, banned domains, and promoted domains.

Exact route names may be adjusted for consistency.

### 18.4. API Security

- Administrative endpoints require authentication.
- Public statistics and search endpoints do not require authentication.
- Do not enable unnecessary cross-origin access.
- Use safe SQL parameter binding.
- Validate input types and supported operations.
- Return appropriate HTTP error codes.
- Do not expose internal database errors or secrets to clients.

Do not implement application-level IP rate limiting, CAPTCHA, or a custom anti-abuse subsystem.

These responsibilities belong to external deployment infrastructure.

---

## 19. External HTTP Security and Deployment

### 19.1. Reverse Proxy

The application is expected to operate behind a reverse proxy such as Caddy.

The reverse proxy is responsible for:

- Public HTTPS.
- HTTP/3 where supported.
- Public-facing TLS configuration.
- HTTP rate limiting.
- Request flooding protection.
- Appropriate proxy headers.
- Public access control where configured.

Cloudflare may additionally provide upstream protection.

The application must not depend on Cloudflare-specific functionality.

### 19.2. Rate Limiting

HTTP API and website request rate limiting is handled externally.

The project must not reimplement general-purpose HTTP abuse management inside Rust.

The README should explain how to configure appropriate limits for:

- Search requests.
- Login attempts.
- Administration endpoints.
- General API requests.

Stock Caddy does not necessarily include a rate-limiting handler; documentation must distinguish standard Caddy functionality from optional modules or separate rate-limiting services.

Do not provide a full Cloudflare setup tutorial.

### 19.3. Origin Protection

Document conventional deployment precautions:

- Restrict direct access to the application origin.
- Expose only required ports.
- Use appropriate reverse-proxy forwarding configuration.
- Keep the database in persistent, protected storage.
- Restrict access to the administrative Unix socket.
- Apply suitable filesystem and service permissions.
- Keep public-facing application dependencies updated.

Do not add a custom firewall management system to SSSE.

### 19.4. Search Engine Indexing

The SSSE website itself must not be indexed by external search engines.

Serve appropriate application-level robots directives and noindex headers or metadata.

This restriction applies to SSSE's own public website, not the websites crawled by SSSE.

---

## 20. Runtime and Resource Management

### 20.1. Responsiveness

Search functionality must remain responsive during crawling.

Avoid:

- Blocking HTTP request handlers with database operations.
- Long database write locks.
- Excessive worker creation.
- Holding large numbers of page bodies in memory.
- Downloading unnecessary binary resources.
- Unbounded in-memory crawl queues.

The SQLite-backed queue is the persistent source of pending crawl work.

### 20.2. Shutdown

Handle termination signals properly.

On graceful shutdown:

- Stop accepting new crawl tasks.
- Cancel or finish active operations safely.
- Release resources.
- Preserve the published index.
- Ensure incomplete staging data cannot be published.

On subsequent startup, clean up incomplete crawling state.

### 20.3. Error Handling

Use structured Rust error handling.

Avoid unnecessary panics and unchecked operations.

Record useful errors through `tracing`.

Logging should be informative without exposing credentials, session tokens, or sensitive page content.

Do not add external telemetry, analytics, or error-reporting platforms.

### 20.4. Database Maintenance

Use appropriate SQLite pragmas and maintenance operations.

Keep database integrity and search-index consistency as priorities.

Avoid automatic operations that would block search requests unnecessarily.

Database schema migrations must support future application upgrades without silently destroying persistent administrative settings or historical aggregate statistics.

---

## 21. README Requirements

The README must explain:

1. What Super Simple Search Engine does.
2. Its architecture and dependencies.
3. Its crawling and indexing behavior.
4. Mandatory robots compliance.
5. How domain popularity is calculated.
6. The limitations of JavaScript-free crawling.
7. How excluded URLs and content bans work.
8. How to install and run it using Nix.
9. How to use the GitHub-hosted flake.
10. How to add the flake as a NixOS configuration input.
11. How to enable the NixOS module.
12. How to run the application in a NixOS container.
13. How to use `nix-shell` for development.
14. How to build and run it outside NixOS.
15. How to initialize or change the administrator password.
16. How to configure seed URLs.
17. How to control the crawler through the CLI.
18. How crawl cycles and index replacement work.
19. How to back up the SQLite database consistently.
20. How to deploy behind a reverse proxy, using Caddy as an example.
21. How to configure external HTTP rate limiting.
22. How DNS and network restrictions affect reachable crawl destinations.
23. How to configure persistent storage and appropriate service permissions.
24. How to use the documented public API.
25. How to contribute to the project.

No GitHub Actions configuration is required.

Do not include instructions that depend on a proprietary hosting provider.

Cloudflare configuration is outside the documentation's scope.

---

## 22. Testing Requirements

Implement automated tests for critical application behavior.

### 22.1. URL and Domain Tests

Test:

- URL normalization and deduplication.
- HTTPS-only filtering.
- IP-literal rejection.
- Registrable-domain identification.
- Subdomain handling.
- Unsupported file identification.
- Redirect handling.
- Maximum crawl depth.

### 22.2. Robots Tests

Test:

- Allow and Disallow matching.
- Different robots rules for subdomains.
- Redirect chains up to five hops.
- Unavailable and unreachable robots files.
- Crawl-delay application.
- Robots caching and refresh.
- Removal of newly disallowed queued URLs.
- Noindex and nofollow behavior.

### 22.3. Crawler Tests

Test:

- Worker concurrency.
- FIFO refill thresholds.
- FIFO capacity derived from worker count.
- Worker delays.
- Shared domain cooldowns.
- Atomic domain timer reservation.
- Queue deduplication.
- Crash-safe queue handling.
- Seed-only crawl initialization.
- Stop and restart operations.
- Prevention of overlapping crawls.

Use controlled local test fixtures rather than depending on live third-party websites.

### 22.4. Index Tests

Test:

- HTML metadata extraction.
- Content-size rejection.
- Binary and download rejection.
- Banned-word matching modes.
- Banned-domain exclusion.
- Popularity counting for every reference.
- BM25 search.
- Domain filtering.
- Quoted queries and exclusions.
- Promoted-domain ordering.
- Image, video, and news classification.
- Publication-date extraction.

### 22.5. Database Tests

Test:

- Schema migrations.
- Transaction rollback.
- Successful index replacement.
- Retention of the previous index after failure.
- Preservation of persistent settings.
- Refused-URL metadata.
- FTS5 index consistency.
- Concurrent search and staging writes.
- Historical statistics snapshots.

### 22.6. Frontend Tests

Test or verify:

- Search navigation.
- Category tabs.
- Pagination.
- Theme persistence.
- System theme detection.
- Custom background storage and removal.
- Responsive image and video grids.
- Keyboard navigation.
- Accessible focus indicators.
- Settings authentication.
- Password changes.
- Statistics graphs.

---

## 23. Configuration Defaults Summary

| Configuration | Default | User configurable |
|---|---|---|
| Crawl interval | 7 days after success | Yes |
| Worker count | Half CPU threads, minimum 1 | Yes |
| Worker request delay | 1 second | Yes |
| Domain request delay | 1 second | Yes |
| FIFO size | Workers × 2 | No |
| FIFO refill check | 1 second | Yes |
| Maximum domain crawl depth | 10 | Yes |
| Maximum extracted text | 100 KiB | Yes |
| Maximum HTML response | 1 MiB | Yes |
| Seed URLs | Empty | Yes |
| Banned domains | Empty | Yes |
| Banned words | Empty | Yes |
| Promoted domains | Empty | Yes |
| Robots compliance | Always enabled | No |
| Robots cache refresh | 24 hours, subject to protocol rules | No |
| Normal page redirects | Not followed | No |
| Normal page protocol | HTTPS only | No |
| Direct IP URLs | Rejected | No |
| JavaScript rendering | Disabled | No |
| Pages per search page | 10 | Browser preference |
| Images per search page | 18 | Browser preference |
| Videos per search page | 18 | Browser preference |
| News per search page | 10 | Browser preference |
| Theme | Follow System | Browser preference |
| Background | None | Browser preference |
| Decorative color lightness | 0.5 | No |
| Decorative color chroma | 0.05 | No |
| Decorative hue | Random per refresh | No |

Reasonable validation ranges must be implemented for configurable numeric values.

Zero disables the maximum depth, extracted-text-size, and downloaded-body-size
limits (unlimited). Zero worker/domain delay means no operator-imposed wait;
website restrictions remain mandatory. Zero workers selects the automatic worker
count, zero FIFO refill interval checks continuously, and zero automatic crawl
interval disables scheduling. These special meanings must be documented in the UI.

Do not add arbitrary maximum limits to the total crawl size, total indexed pages, or total number of domains.

---

## 24. Explicit Non-Goals

The following are not part of the project:

- Implementing PageRank.
- Implementing a custom search database.
- Implementing a separate TF-IDF engine.
- Running Elasticsearch or another search service.
- Running Redis.
- Adding a dedicated distributed task queue.
- Running multiple crawler instances.
- Adding a frontend framework.
- Adding Node.js as a frontend build requirement.
- Running headless Chromium or another browser engine.
- Executing website JavaScript.
- Downloading or indexing PDFs and ordinary documents.
- Downloading video files.
- Storing image binaries.
- Proxying remote media.
- Implementing HTTP IP-based rate limiting inside Rust.
- Implementing CAPTCHA or an anti-bot service.
- Implementing Cloudflare-specific integration.
- Adding GitHub Actions.
- Distributing precompiled executable files in the repository.
- Providing user accounts or multi-user roles.
- Implementing complex website trust or reputation scoring.
- Retaining old search-index generations.
- Reusing previously indexed URLs when beginning a new crawl.
- Maintaining full historical per-page crawl data.
- Indexing SSSE's own website through public search engines.
- Building an extensive external configuration system.

Do not introduce these features without explicit approval.

---

## 25. Implementation Guidelines for the AI Agent

### 25.1. General

Implement the specification faithfully.

Prioritize clarity and correctness over abstraction.

Use established libraries rather than reinventing standard functionality.

Prefer simple, understandable Rust modules.

Avoid unnecessary dependency additions.

Do not add unrelated features.

Do not silently change crawler behavior, database lifetime rules, or search-ranking semantics.

### 25.2. Priority Order

When deciding implementation details, prioritize:

1. Correctness of crawling restrictions.
2. Integrity of the published index and persistent settings.
3. Correct URL discovery and popularity counting.
4. Reliable concurrency and scheduling.
5. Search relevance and correctness.
6. Low operational complexity.
7. Resource efficiency.
8. Frontend usability and accessibility.
9. Maintainability and clean code.

### 25.3. Recommended Implementation Order

1. Rust project skeleton and Nix development environment.
2. Database initialization, schema, and migrations.
3. Application settings and persistent administrative data.
4. Rust HTTP server and static frontend serving.
5. Authentication and CLI control socket.
6. Crawl queue, worker pool, and cooldown management.
7. Robots handling and URL validation.
8. HTML extraction and content filters.
9. Page, image, video, and news indexing.
10. Domain popularity calculation.
11. FTS5 search and BM25 ranking.
12. Crawl completion and atomic index replacement.
13. Public search API.
14. Search frontend and browser preferences.
15. Statistics collection, API, and graphs.
16. Settings frontend and crawler controls.
17. NixOS module and deployment integration.
18. Tests, documentation, and final validation.

### 25.4. Completion Criteria

The implementation is complete when:

- `nix develop` and `nix-shell` provide working development environments.
- The project builds using stable Rust.
- The flake supports `nix run` and package installation.
- The application can run inside a NixOS container.
- The server initializes and uses one SQLite database.
- The frontend is served directly by the Rust application.
- Administrator authentication and password changes work.
- Seed URLs can be configured.
- Crawling runs concurrently with correct domain timers.
- Robots restrictions are always enforced.
- HTTPS-only page crawling is respected.
- Banned content is excluded and refusal reasons are recorded.
- Every eligible discovered link reference contributes to domain popularity.
- Pages, Images, Videos, and News are searchable.
- FTS5 and BM25 ranking operate correctly.
- Promoted domains are ranked ahead of ordinary matching results.
- Search remains usable while crawling.
- Successful crawls completely replace the old index.
- Failed or stopped crawls preserve the old index.
- The CLI controls the existing application process.
- Statistics and historical graphs work.
- Theme, background, and pagination preferences persist in the browser.
- The UI is responsive and targets WCAG 2.2 AA.
- The public API is documented.
- The README explains Nix installation and external reverse-proxy rate limiting.
- No prohibited or unnecessary services have been introduced.

---

## 26. Technical References

The implementation should refer to official specifications and documentation where applicable.

- [Rust documentation](https://doc.rust-lang.org/)
- [Tokio documentation](https://tokio.rs/)
- [Axum documentation](https://docs.rs/axum/latest/axum/)
- [rusqlite documentation](https://docs.rs/rusqlite/latest/rusqlite/)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite WAL](https://www.sqlite.org/wal.html)
- [RFC 9309 — Robots Exclusion Protocol](https://www.rfc-editor.org/rfc/rfc9309.html)
- [The Anatomy of a Large-Scale Hypertextual Web Search Engine](https://research.google/pubs/the-anatomy-of-a-large-scale-hypertextual-web-search-engine/)
- [Nix flakes documentation](https://nix.dev/concepts/flakes)
- [NixOS documentation](https://nixos.org/learn/)
- [MDN Web Docs](https://developer.mozilla.org/)
- [WCAG 2.2](https://www.w3.org/TR/WCAG22/)
- [OWASP Session Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html)

Use maintained stable versions of the selected technologies.

The objective is not to reproduce an industrial-scale commercial search engine.

**The objective is to implement a genuinely simple, functional, extensible search engine without unnecessary architectural complexity.**
