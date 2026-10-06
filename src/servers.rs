pub struct TestServer {
    pub id: &'static str,
    pub label: &'static str,
    pub ping_url: &'static str,
    /// Tried in order until one succeeds.
    pub download_urls: &'static [&'static str],
    pub upload_url: &'static str,
}

/// The first entry is the default.
pub static TEST_SERVERS: &[TestServer] = &[TestServer {
    id: "cloudflare",
    label: "Cloudflare (nearest edge)",
    ping_url: "https://speed.cloudflare.com/__down?bytes=0",
    download_urls: &["https://speed.cloudflare.com/__down?bytes=26214400"],
    upload_url: "https://speed.cloudflare.com/__up",
}];
