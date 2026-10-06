# net-speed

Small Rust CLI that measures internet ping, download and upload speed against a fixed list of public test servers.

## Usage

```sh
cargo run --release
cargo run --release -- --help
```

Tests run against Cloudflare's speed-test endpoints (`speed.cloudflare.com`); anycast routes to the nearest edge, so there is no server to pick. `--server <id>` selects another entry if you add one to `src/servers.rs`.

Example output:

```
Server:         Cloudflare (nearest edge) (cloudflare)
Edge:           SIN
Ping (median):    123.45 ms
Download:         95.10 Mbps
Upload:           20.33 Mbps
```

After the three tests it also prints:

- **Speed graph:** a sparkline of download speed sampled every ~200 ms.
- **What this means for you:** estimated time for `npm install`, `docker pull`, `git clone` of the Linux kernel, and whether a 4K stream fits, derived from the download speed.
- **Developer services:** latency to npm, crates.io, PyPI, GitHub, Docker Hub and ghcr.io (concurrent HEAD requests, warm connection).

Each phase prints `failed (reason)` instead of aborting if it errors.

## How it works

Code is in `src/main.rs`; the server table is in `src/servers.rs`. Uses `tokio` + `reqwest` (rustls, HTTP/2) with a 10s connect / 20s total timeout.

1. **Server selection** (`choose_server`): `--server <id>` is looked up in `TEST_SERVERS` (`src/servers.rs`); the default is its first entry.
2. **Ping** (`profile_ping_url`): one warm-up GET (excluded), then 5 sequential GETs of the server's `ping_url` (`__down?bytes=0`); reports the median full-request time in ms (a rough latency figure, not ICMP).
3. **Download** (`profile_download`): opens 4 parallel streams from `__down` and stops after 25 MiB in total or 10 s, whichever comes first. Tries each URL in `download_urls` in order. Speed = bytes × 8 / elapsed time, with live progress.
4. **Upload** (`profile_upload`): builds a 2 MiB zero-filled payload in memory (not timed), then POSTs it to the server's `upload_url` (`__up`) and times the request.

## Adding a server

Add a `TestServer { id, label, ping_url, download_urls, upload_url }` entry to `TEST_SERVERS` in `src/servers.rs`. `--help` lists it automatically.

## Caveats

- Results depend on Cloudflare's speed-test endpoints, which are public but not an officially documented API.
- Download and upload use small samples (and upload a single connection), so results are approximate and will read lower than multi-stream tools like speedtest.net.
