use futures_util::{StreamExt, future::join_all, stream::select_all};
use reqwest::{Client, header::HeaderMap};
use std::env;
use std::error::Error;
use std::future::Future;
use std::io::{self, Write};
use std::time::{Duration, Instant};

mod servers;
use servers::{TEST_SERVERS, TestServer};

const DOWNLOAD_STREAMS: usize = 4;
const DOWNLOAD_MAX_TIME: Duration = Duration::from_secs(10);

fn mbps(bytes: u64, elapsed: Duration) -> f64 {
    (bytes as f64 * 8.0) / elapsed.as_secs_f64() / 1_000_000.0
}

const DEV_TARGETS: [(&str, &str); 6] = [
    ("npm", "https://registry.npmjs.org/"),
    ("crates.io", "https://index.crates.io/config.json"),
    ("PyPI", "https://pypi.org/simple/"),
    ("GitHub", "https://github.com/"),
    ("Docker Hub", "https://registry-1.docker.io/v2/"),
    ("ghcr.io", "https://ghcr.io/v2/"),
];

// (what, size in MB)
const DEV_TASKS: [(&str, f64); 3] = [
    ("npm install (~150 MB)", 150.0),
    ("docker pull (~400 MB image)", 400.0),
    ("git clone linux kernel (~4 GB)", 4096.0),
];

fn sparkline(samples: &[f64]) -> String {
    let max = samples.iter().cloned().fold(0.0, f64::max);
    if max <= 0.0 {
        return String::new();
    }
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    samples
        .iter()
        .map(|x| BARS[((x / max) * 7.0).round() as usize])
        .collect()
}

fn fmt_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("~{:.0}s", secs.max(1.0))
    } else {
        format!("~{}m {:02}s", (secs / 60.0) as u64, (secs % 60.0) as u64)
    }
}

fn print_verdict(download_mbps: f64) {
    println!("\nWhat this means for you:");
    for (what, mb) in DEV_TASKS {
        println!("  {what:<34} {}", fmt_duration(mb * 8.0 / download_mbps));
    }
    let stream = if download_mbps >= 25.0 { "✓" } else { "✗" };
    println!("  {:<34} {stream}", "4K stream (25 Mbps)");
}

async fn profile_dev_targets(client: &Client) {
    println!("\nDeveloper services (latency):");
    let results = join_all(DEV_TARGETS.iter().map(|(name, url)| async move {
        // warm-up request so the timed one excludes DNS/TLS setup
        let _ = client.head(*url).send().await;
        let started = Instant::now();
        let res = client.head(*url).send().await;
        (
            *name,
            res.map(|_| started.elapsed().as_secs_f64() * 1_000.0),
        )
    }))
    .await;

    for (name, res) in results {
        match res {
            Ok(ms) => println!("  {name:<12} {ms:>7.0} ms"),
            Err(_) => println!("  {name:<12} unreachable"),
        }
    }
}

fn fail_reason(err: &(dyn Error + 'static)) -> String {
    match err
        .downcast_ref::<reqwest::Error>()
        .and_then(|e| e.status())
    {
        Some(code) => format!("status {}", code.as_u16()),
        None => err.to_string(),
    }
}

fn print_progress(label: &str, transferred: u64, total: u64, started: Instant) {
    let percent = if total == 0 {
        0.0
    } else {
        (transferred as f64 / total as f64) * 100.0
    };
    let speed = mbps(transferred, started.elapsed());

    print!(
        "\r\x1b[2K{label}: {:>6.2}% ({:>5.2}/{:>5.2} MiB) {:>6.2} Mbps",
        percent.min(100.0),
        transferred as f64 / (1024.0 * 1024.0),
        total as f64 / (1024.0 * 1024.0),
        speed
    );
    let _ = io::stdout().flush();
}

fn print_prepare_progress(label: &str, transferred: u64, total: u64) {
    let percent = if total == 0 {
        0.0
    } else {
        (transferred as f64 / total as f64) * 100.0
    };

    print!(
        "\r\x1b[2K{label}: {:>6.2}% ({:>5.2}/{:>5.2} MiB)",
        percent.min(100.0),
        transferred as f64 / (1024.0 * 1024.0),
        total as f64 / (1024.0 * 1024.0),
    );
    let _ = io::stdout().flush();
}

struct Latency {
    min: f64,
    median: f64,
    p90: f64,
    max: f64,
    jitter: f64, // mean absolute difference between consecutive samples
}

fn latency_stats(samples: &[f64]) -> Option<Latency> {
    if samples.is_empty() {
        return None;
    }
    let jitter = if samples.len() < 2 {
        0.0
    } else {
        samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (samples.len() - 1) as f64
    };
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    Some(Latency {
        min: sorted[0],
        median: sorted[n / 2],
        p90: sorted[((n as f64 * 0.9).ceil() as usize).clamp(1, n) - 1],
        max: sorted[n - 1],
        jitter,
    })
}

fn print_latency(label: &str, latency: Option<&Latency>) {
    match latency {
        Some(l) => println!(
            "{label:<15}{:>8.2} ms  (min {:.1} / p90 {:.1} / max {:.1}, jitter {:.1})",
            l.median, l.min, l.p90, l.max, l.jitter
        ),
        None => println!("{label:<15}n/a"),
    }
}

/// Connection details Cloudflare puts on every `__down` response; missing ones are skipped.
fn print_connection(headers: &HeaderMap) {
    let get = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    if let Some(ip) = get("cf-meta-ip") {
        println!("Your IP:        {ip}");
    }
    if let Some(asn) = get("asn") {
        println!("Network:        AS{asn}");
    }
    let place: Vec<_> = [get("city"), get("country")]
        .into_iter()
        .flatten()
        .collect();
    if !place.is_empty() {
        println!("Location:       {}", place.join(", "));
    }
    println!("Edge:           {}", get("colo").unwrap_or("unknown"));
}

/// One warm-up GET (excluded from stats), then `samples` timed GETs.
async fn profile_ping_url(
    client: &Client,
    samples: usize,
    ping_url: &str,
) -> Result<(Latency, HeaderMap), Box<dyn Error>> {
    let headers = client
        .get(ping_url)
        .send()
        .await?
        .error_for_status()?
        .headers()
        .clone();

    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        client.get(ping_url).send().await?.error_for_status()?;
        times.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    Ok((latency_stats(&times).ok_or("no ping samples")?, headers))
}

async fn probe_loop(client: &Client, ping_url: &str, times: &mut Vec<f64>) {
    let mut warm = false; // first probe may pay for a new connection, so drop it
    loop {
        let started = Instant::now();
        if client
            .get(ping_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .is_ok()
        {
            if warm {
                times.push(started.elapsed().as_secs_f64() * 1_000.0);
            }
            warm = true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Runs `work` while pinging in the background; returns its output and the latency seen under load.
async fn with_loaded_latency<T>(
    client: &Client,
    ping_url: &str,
    work: impl Future<Output = T>,
) -> (T, Option<Latency>) {
    let mut times = Vec::new();
    let out = tokio::select! {
        out = work => out,
        _ = probe_loop(client, ping_url, &mut times) => unreachable!(),
    };
    (out, latency_stats(&times))
}

fn find_server_by_id(server_id: &str) -> Option<&'static TestServer> {
    TEST_SERVERS.iter().find(|server| server.id == server_id)
}

fn parse_server_choice() -> Result<Option<String>, String> {
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--server" {
            let value = args
                .next()
                .ok_or_else(|| String::from("missing value for --server"))?;
            return Ok(Some(value.to_lowercase()));
        }

        if arg.starts_with("--server=") {
            let value = arg.trim_start_matches("--server=").to_lowercase();
            if value.is_empty() {
                return Err(String::from("missing value for --server"));
            }
            return Ok(Some(value));
        }

        if arg == "-h" || arg == "--help" {
            println!("{}", usage());
            std::process::exit(0);
        }
    }

    Ok(None)
}

fn usage() -> String {
    let ids: Vec<_> = TEST_SERVERS.iter().map(|s| s.id).collect();
    format!("Usage: net-speed [--server {}]", ids.join("|"))
}

fn choose_server(choice: Option<String>) -> Result<&'static TestServer, String> {
    match choice.as_deref() {
        None | Some("nearest") => Ok(&TEST_SERVERS[0]),
        Some(id) => find_server_by_id(id).ok_or_else(|| format!("unsupported server '{id}'")),
    }
}

async fn profile_download(
    client: &Client,
    download_urls: &[&str],
    target_bytes: u64,
) -> Result<(u64, Duration, Vec<f64>), Box<dyn Error>> {
    let mut last_error = String::from("no download URL attempted");

    for url in download_urls {
        // one connection can't fill a fast link, so read several streams at once
        let responses = join_all((0..DOWNLOAD_STREAMS).map(|_| client.get(*url).send())).await;
        match responses.into_iter().collect::<Result<Vec<_>, _>>() {
            Ok(responses) => {
                let mut streams = Vec::with_capacity(responses.len());
                for response in responses {
                    match response.error_for_status() {
                        Ok(ok) => streams.push(ok.bytes_stream()),
                        Err(err) => last_error = fail_reason(&err),
                    }
                }
                if streams.len() < DOWNLOAD_STREAMS {
                    continue;
                }

                let mut stream = select_all(streams);
                let started = Instant::now();
                let mut downloaded: u64 = 0;
                let mut last_progress_tick = Instant::now();
                let mut last_bytes: u64 = 0;
                let mut samples: Vec<f64> = Vec::new();

                print_progress("Downloading", downloaded, target_bytes, started);

                let mut stream_failed = false;

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            downloaded += chunk.len() as u64;

                            let tick = last_progress_tick.elapsed();
                            if tick >= Duration::from_millis(200) || downloaded >= target_bytes {
                                if tick >= Duration::from_millis(100) {
                                    samples.push(mbps(downloaded - last_bytes, tick));
                                }
                                last_bytes = downloaded;
                                print_progress("Downloading", downloaded, target_bytes, started);
                                last_progress_tick = Instant::now();
                            }

                            if downloaded >= target_bytes || started.elapsed() >= DOWNLOAD_MAX_TIME
                            {
                                break;
                            }
                        }
                        Err(err) => {
                            last_error = fail_reason(&err);
                            stream_failed = true;
                            break;
                        }
                    }
                }

                if stream_failed {
                    continue;
                }

                if downloaded > 0 {
                    return Ok((downloaded, started.elapsed(), samples));
                }

                last_error = String::from("no bytes downloaded");
            }
            Err(err) => {
                last_error = fail_reason(&err);
            }
        }
    }

    Err(last_error.into())
}

async fn profile_upload(
    client: &Client,
    upload_url: &str,
    upload_bytes: usize,
) -> Result<(u64, Duration), Box<dyn Error>> {
    let chunk_size = 64 * 1024;
    let chunk = vec![0_u8; chunk_size];
    let mut payload = Vec::with_capacity(upload_bytes);
    let mut uploaded: usize = 0;
    let mut last_progress_tick = Instant::now();

    print_prepare_progress("Preparing upload", uploaded as u64, upload_bytes as u64);

    while uploaded < upload_bytes {
        let remaining = upload_bytes - uploaded;
        let write_size = remaining.min(chunk_size);
        payload.extend_from_slice(&chunk[..write_size]);
        uploaded += write_size;

        if last_progress_tick.elapsed() >= Duration::from_millis(120) || uploaded == upload_bytes {
            print_prepare_progress("Preparing upload", uploaded as u64, upload_bytes as u64);
            last_progress_tick = Instant::now();
        }
    }

    print!("\r\x1b[2KUploading request...");
    let _ = io::stdout().flush();
    let started = Instant::now();

    client
        .post(upload_url)
        .body(payload)
        .send()
        .await?
        .error_for_status()?;

    let elapsed = started.elapsed();

    Ok((upload_bytes as u64, elapsed))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("net-speed/", env!("CARGO_PKG_VERSION")))
        .build()?;

    println!("net-speed: internet speed profiling\n");

    let server_choice = match parse_server_choice() {
        Ok(choice) => choice,
        Err(err) => {
            println!("Server selection: failed ({err})");
            println!("{}", usage());
            return Ok(());
        }
    };

    let server = match choose_server(server_choice) {
        Ok(server) => server,
        Err(err) => {
            println!("Server selection: failed ({err})");
            return Ok(());
        }
    };

    let ping = profile_ping_url(&client, 10, server.ping_url).await;
    let (download, loaded_down) = with_loaded_latency(
        &client,
        server.ping_url,
        profile_download(&client, server.download_urls, 25 * 1024 * 1024),
    )
    .await;
    let (upload, loaded_up) = with_loaded_latency(
        &client,
        server.ping_url,
        profile_upload(&client, server.upload_url, 2 * 1024 * 1024),
    )
    .await;
    print!("\r\x1b[2K"); // erase the last progress line

    println!("Server:         {} ({})", server.label, server.id);
    if let Ok((_, headers)) = &ping {
        print_connection(headers);
    }

    println!();
    match &ping {
        Ok((latency, _)) => print_latency("Ping (median):", Some(latency)),
        Err(err) => println!("Ping (median): failed ({})", fail_reason(err.as_ref())),
    }

    let mut download_mbps = None;
    match &download {
        Ok((bytes, elapsed, samples)) => {
            let speed = mbps(*bytes, *elapsed);
            println!("Download:      {:>8.2} Mbps", speed);
            print_latency("Loaded (down):", loaded_down.as_ref());
            println!("Speed graph:   {}", sparkline(samples));
            download_mbps = Some(speed);
        }
        Err(err) => println!("Download:      failed ({})", fail_reason(err.as_ref())),
    }

    match &upload {
        Ok((bytes, elapsed)) => {
            println!("Upload:        {:>8.2} Mbps", mbps(*bytes, *elapsed));
            print_latency("Loaded (up):", loaded_up.as_ref());
        }
        Err(err) => println!("Upload:        failed ({})", fail_reason(err.as_ref())),
    }

    if let Some(speed) = download_mbps {
        print_verdict(speed);
    }
    profile_dev_targets(&client).await;

    Ok(())
}
