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
Your IP:        203.0.113.7
Network:        AS64500
Location:       Bangkok, TH
Edge:           BKK
Ping (median):    12.40 ms  (min 10.9 / p90 15.2 / max 18.0, jitter 1.3)
Download:         95.10 Mbps
Loaded (down):    35.20 ms  (min 14.0 / p90 60.1 / max 80.3, jitter 9.8)
Upload:           20.33 Mbps
Loaded (up):      48.00 ms  (min 15.1 / p90 90.2 / max 95.0, jitter 12.0)
```

Latency lines show the median, with min / p90 / max and jitter (mean absolute difference between consecutive samples). **Loaded** is latency measured while the download or upload is running; a big jump over the idle ping means bufferbloat.

After the three tests it also prints:

- **Speed graph:** a sparkline of download speed sampled every ~200 ms.
- **What this means for you:** estimated time for `npm install`, `docker pull`, `git clone` of the Linux kernel, and whether a 4K stream fits, derived from the download speed.
- **Developer services:** latency to npm, crates.io, PyPI, GitHub, Docker Hub and ghcr.io (concurrent HEAD requests, warm connection).

Each phase prints `failed (reason)` instead of aborting if it errors; HTTP errors show just the status, e.g. `failed (status 429)`. Progress is drawn on one transient line, and the connection info and results are printed together at the end.

## How it works

Code is in `src/main.rs`; the server table is in `src/servers.rs`. Uses `tokio` + `reqwest` (rustls, HTTP/2) with a 10s connect / 20s total timeout.

1. **Server selection** (`choose_server`): `--server <id>` is looked up in `TEST_SERVERS` (`src/servers.rs`); the default is its first entry.
2. **Ping** (`profile_ping_url`): one warm-up GET (excluded), then 10 sequential GETs of the server's `ping_url` (`__down?bytes=0`); reports median/min/p90/max/jitter in ms. The warm-up response's `cf-meta-ip`, `asn`, `city`, `country` and `colo` headers give the connection info (the ISP name is only on `/meta`, which needs browser headers, so only the ASN is shown) (a rough latency figure, not ICMP).
3. **Download** (`profile_download`): opens 4 parallel streams from `__down` and stops after 25 MiB in total or 10 s, whichever comes first. Tries each URL in `download_urls` in order. Speed = bytes × 8 / elapsed time, with live progress. A background probe (`with_loaded_latency`) pings every 200 ms while it runs.
4. **Upload** (`profile_upload`): builds a 2 MiB zero-filled payload in memory (not timed), then POSTs it to the server's `upload_url` (`__up`) and times the request, with the same background probe running.

## Adding a server

Add a `TestServer { id, label, ping_url, download_urls, upload_url }` entry to `TEST_SERVERS` in `src/servers.rs`. `--help` lists it automatically.

## Caveats

- Results depend on Cloudflare's speed-test endpoints, which are public but not an officially documented API. `__down` requests of >= 10 MB are rate-limited per IP (`429`, `Retry-After` ~50 min), so each download stream asks for 9 MB (4 streams ≈ 36 MB, stopping at 25 MiB). On very fast links this is a short sample.
- Download and upload use small samples (and upload a single connection), so results are approximate and will read lower than multi-stream tools like speedtest.net.
