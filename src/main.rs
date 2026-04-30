use futures_util::StreamExt;
use reqwest::Client;
use std::env;
use std::error::Error;
use std::io::{self, Write};
use std::time::{Duration, Instant};

struct TestServer {
    id: &'static str,
    label: &'static str,
    ping_url: &'static str,
    download_urls: &'static [&'static str],
    upload_url: &'static str,
}

const HK_DOWNLOAD_URLS: [&str; 2] = [
    "https://hkg.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const SG_DOWNLOAD_URLS: [&str; 2] = [
    "https://sgp.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const TH_DOWNLOAD_URLS: [&str; 2] = [
    "https://bkk.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const SYD_DOWNLOAD_URLS: [&str; 2] = [
    "https://syd.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const JP_DOWNLOAD_URLS: [&str; 2] = [
    "https://tyo.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const DE_DOWNLOAD_URLS: [&str; 2] = [
    "https://fra.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const US_DOWNLOAD_URLS: [&str; 2] = [
    "https://nyc.download.datapacket.com/100mb.bin",
    "https://proof.ovh.net/files/100Mb.dat",
];

const TEST_SERVERS: [TestServer; 7] = [
    TestServer {
        id: "hongkong",
        label: "Hong Kong",
        ping_url: "https://hkg.download.datapacket.com/1mb.bin",
        download_urls: &HK_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "singapore",
        label: "Singapore",
        ping_url: "https://sgp.download.datapacket.com/1mb.bin",
        download_urls: &SG_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "thailand",
        label: "Thailand",
        ping_url: "https://bkk.download.datapacket.com/1mb.bin",
        download_urls: &TH_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "sydney",
        label: "Sydney",
        ping_url: "https://syd.download.datapacket.com/1mb.bin",
        download_urls: &SYD_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "japan",
        label: "Japan",
        ping_url: "https://tyo.download.datapacket.com/1mb.bin",
        download_urls: &JP_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "germany",
        label: "Germany",
        ping_url: "https://fra.download.datapacket.com/1mb.bin",
        download_urls: &DE_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
    TestServer {
        id: "us",
        label: "United States",
        ping_url: "https://nyc.download.datapacket.com/1mb.bin",
        download_urls: &US_DOWNLOAD_URLS,
        upload_url: "https://httpbin.org/post",
    },
];

fn mbps(bytes: u64, elapsed: Duration) -> f64 {
    (bytes as f64 * 8.0) / elapsed.as_secs_f64() / 1_000_000.0
}

fn print_progress(label: &str, transferred: u64, total: u64, started: Instant) {
    let percent = if total == 0 {
        0.0
    } else {
        (transferred as f64 / total as f64) * 100.0
    };
    let speed = mbps(transferred, started.elapsed());

    print!(
        "\r{label}: {:>6.2}% ({:>5.2}/{:>5.2} MiB) {:>6.2} Mbps",
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
        "\r{label}: {:>6.2}% ({:>5.2}/{:>5.2} MiB)",
        percent.min(100.0),
        transferred as f64 / (1024.0 * 1024.0),
        total as f64 / (1024.0 * 1024.0),
    );
    let _ = io::stdout().flush();
}

async fn profile_ping_url(client: &Client, samples: usize, ping_url: &str) -> Result<f64, Box<dyn Error>> {
    let mut total_ms = 0.0;

    for _ in 0..samples {
        let started = Instant::now();
        client.get(ping_url).send().await?.error_for_status()?;
        total_ms += started.elapsed().as_secs_f64() * 1_000.0;
    }

    Ok(total_ms / samples as f64)
}

fn find_server_by_id(server_id: &str) -> Option<&'static TestServer> {
    TEST_SERVERS
        .iter()
        .find(|server| server.id == server_id)
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
            println!("Usage: net-speed [--server nearest|hongkong|singapore|thailand|sydney|japan|germany|us]");
            println!("Default: --server nearest");
            std::process::exit(0);
        }
    }

    Ok(None)
}

async fn choose_server(client: &Client, choice: Option<String>) -> Result<&'static TestServer, String> {
    let normalized = choice.unwrap_or_else(|| String::from("nearest"));

    let explicit = match normalized.as_str() {
        "hk" => Some("hongkong"),
        "sg" => Some("singapore"),
        "th" => Some("thailand"),
        "au" => Some("sydney"),
        "jp" => Some("japan"),
        "de" => Some("germany"),
        "nearest" => None,
        other => Some(other),
    };

    if let Some(server_id) = explicit {
        return find_server_by_id(server_id)
            .ok_or_else(|| format!("unsupported server '{server_id}'. Use nearest, hongkong, singapore, thailand, sydney, japan, germany, us"));
    }

    let mut best: Option<(&TestServer, f64)> = None;
    for server in &TEST_SERVERS {
        if let Ok(latency) = profile_ping_url(client, 2, server.ping_url).await {
            match best {
                Some((_, best_latency)) if latency >= best_latency => {}
                _ => best = Some((server, latency)),
            }
        }
    }

    best.map(|(server, _)| server)
        .ok_or_else(|| String::from("could not determine nearest server (all probes failed)"))
}

async fn profile_download(client: &Client, download_urls: &[&str], target_bytes: u64) -> Result<(u64, Duration), Box<dyn Error>> {
    let mut last_error = String::from("no download URL attempted");

    for url in download_urls {
        match client.get(*url).send().await {
            Ok(response) => {
                let response = match response.error_for_status() {
                    Ok(ok) => ok,
                    Err(err) => {
                        last_error = format!("{url}: {err}");
                        continue;
                    }
                };

                let mut stream = response.bytes_stream();
                let started = Instant::now();
                let mut downloaded: u64 = 0;
                let mut last_progress_tick = Instant::now();

                print_progress("Downloading", downloaded, target_bytes, started);

                let mut stream_failed = false;

                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(chunk) => {
                            downloaded += chunk.len() as u64;

                            if last_progress_tick.elapsed() >= Duration::from_millis(200)
                                || downloaded >= target_bytes
                            {
                                print_progress("Downloading", downloaded, target_bytes, started);
                                last_progress_tick = Instant::now();
                            }

                            if downloaded >= target_bytes {
                                break;
                            }
                        }
                        Err(err) => {
                            last_error = format!("{url}: {err}");
                            stream_failed = true;
                            break;
                        }
                    }
                }

                if stream_failed {
                    continue;
                }

                if downloaded > 0 {
                    print_progress("Downloading", downloaded, target_bytes, started);
                    println!();
                    return Ok((downloaded, started.elapsed()));
                }

                last_error = format!("{url}: no bytes downloaded");
            }
            Err(err) => {
                last_error = format!("{url}: {err}");
            }
        }
    }

    Err(last_error.into())
}

async fn profile_upload(client: &Client, upload_url: &str, upload_bytes: usize) -> Result<(u64, Duration), Box<dyn Error>> {
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

    println!();
    println!("Uploading request...");
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
        .build()?;

    println!("net-speed: internet speed profiling\n");

    let server_choice = match parse_server_choice() {
        Ok(choice) => choice,
        Err(err) => {
            println!("Server selection: failed ({err})");
            println!("Usage: net-speed [--server nearest|hongkong|singapore|thailand|sydney|japan|germany|us]");
            return Ok(());
        }
    };

    let server = match choose_server(&client, server_choice).await {
        Ok(server) => server,
        Err(err) => {
            println!("Server selection: failed ({err})");
            return Ok(());
        }
    };

    println!("Server:         {} ({})", server.label, server.id);

    match profile_ping_url(&client, 5, server.ping_url).await {
        Ok(avg_ping_ms) => println!("Ping (avg):    {:>8.2} ms", avg_ping_ms),
        Err(err) => println!("Ping (avg):    failed ({err})"),
    }

    match profile_download(&client, server.download_urls, 8 * 1024 * 1024).await {
        Ok((bytes, elapsed)) => println!("Download:      {:>8.2} Mbps", mbps(bytes, elapsed)),
        Err(err) => println!("Download:      failed ({err})"),
    }

    match profile_upload(&client, server.upload_url, 2 * 1024 * 1024).await {
        Ok((bytes, elapsed)) => println!("Upload:        {:>8.2} Mbps", mbps(bytes, elapsed)),
        Err(err) => println!("Upload:        failed ({err})"),
    }

    Ok(())
}
