use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use image::ImageFormat;
use rsclip_core::RsclipPaths;
use scraper::{Html, Selector};
use url::Url;

const HTTP_TIMEOUT: Duration = Duration::from_millis(1200);
const HTML_CAP_BYTES: usize = 64 * 1024;
const IMAGE_CAP_BYTES: usize = 256 * 1024;
const MAX_IMAGE_ATTEMPTS: usize = 4;
const USER_AGENT: &str = "rsclipd/0.1 favicon-cache";

#[derive(Clone)]
struct IconCandidate {
    url: Url,
    priority: u8,
}

pub fn fetch_and_cache_domain(paths: &RsclipPaths, domain: &str) -> Result<()> {
    if !is_fetchable_domain(domain) {
        write_miss(paths, domain)?;
        bail!("skipping non-public favicon domain: {domain}");
    }

    let origin = Url::parse(&format!("https://{domain}"))
        .with_context(|| format!("building favicon origin for {domain}"))?;
    fetch_and_cache_from_origin(paths, domain, &origin)
}

fn fetch_and_cache_from_origin(paths: &RsclipPaths, domain: &str, origin: &Url) -> Result<()> {
    match fetch_png(origin) {
        Ok(png) => {
            save_icon(paths, domain, &png)?;
            let miss_path = rsclip_core::favicons::miss_path(paths, domain);
            match fs::remove_file(&miss_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| format!("removing {}", miss_path.display()));
                }
            }
            Ok(())
        }
        Err(err) => {
            write_miss(paths, domain)?;
            Err(err)
        }
    }
}

fn fetch_png(origin: &Url) -> Result<Vec<u8>> {
    if !matches!(origin.scheme(), "http" | "https") {
        bail!("unsupported favicon origin scheme: {}", origin.scheme());
    }

    let agent = agent();
    let favicon_url = origin.join("/favicon.ico")?;
    if let Ok(bytes) = fetch_bytes(&agent, favicon_url.as_str(), IMAGE_CAP_BYTES)
        && let Ok(png) = normalize_png(&bytes)
    {
        return Ok(png);
    }

    let html = fetch_bytes(&agent, origin.as_str(), HTML_CAP_BYTES)
        .context("fetching homepage HTML for favicon discovery")?;
    let html = String::from_utf8_lossy(&html);
    let mut candidates = discover_icon_candidates(origin, &html);
    candidates.sort_by_key(|candidate| candidate.priority);
    candidates.truncate(MAX_IMAGE_ATTEMPTS);
    if candidates.is_empty() {
        bail!("homepage HTML had no usable favicon candidates");
    }

    for candidate in candidates {
        let Ok(bytes) = fetch_bytes(&agent, candidate.url.as_str(), IMAGE_CAP_BYTES) else {
            continue;
        };
        if let Ok(png) = normalize_png(&bytes) {
            return Ok(png);
        }
    }

    bail!("all favicon image candidates failed")
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .timeout_per_call(Some(HTTP_TIMEOUT))
        .timeout_connect(Some(HTTP_TIMEOUT))
        .timeout_recv_response(Some(HTTP_TIMEOUT))
        .timeout_recv_body(Some(HTTP_TIMEOUT))
        .user_agent(USER_AGENT)
        .build();
    ureq::Agent::new_with_config(config)
}

fn fetch_bytes(agent: &ureq::Agent, url: &str, cap: usize) -> Result<Vec<u8>> {
    let mut response = agent
        .get(url)
        .call()
        .with_context(|| format!("fetching {url}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit((cap + 1) as u64)
        .read_to_vec()
        .with_context(|| format!("reading response body from {url}"))?;
    if bytes.len() > cap {
        bail!("response from {url} exceeded {} bytes", cap);
    }
    Ok(bytes)
}

fn discover_icon_candidates(origin: &Url, html: &str) -> Vec<IconCandidate> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("link").expect("static selector should parse");
    let mut candidates = Vec::new();

    for element in document.select(&selector) {
        let Some(rel) = element.value().attr("rel") else {
            continue;
        };
        let rel_lower = rel.to_ascii_lowercase();
        if !rel_lower.contains("icon") {
            continue;
        }

        let Some(href) = element.value().attr("href") else {
            continue;
        };
        if is_clearly_svg(href) {
            continue;
        }

        let Ok(url) = origin.join(href) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") || is_clearly_svg(url.path()) {
            continue;
        }

        candidates.push(IconCandidate {
            url,
            priority: candidate_priority(&rel_lower),
        });
    }

    candidates
}

fn candidate_priority(rel: &str) -> u8 {
    let tokens = rel.split_ascii_whitespace().collect::<Vec<_>>();
    if tokens == ["icon"] {
        0
    } else if tokens.contains(&"shortcut") && tokens.contains(&"icon") {
        1
    } else if tokens.contains(&"apple-touch-icon") {
        2
    } else {
        3
    }
}

fn is_clearly_svg(value: &str) -> bool {
    value
        .split('?')
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase()
        .ends_with(".svg")
}

fn is_fetchable_domain(domain: &str) -> bool {
    let labels = domain.split('.').collect::<Vec<_>>();
    let Some(tld) = labels.last() else {
        return false;
    };

    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        && tld.len() >= 2
        && tld.bytes().all(|byte| byte.is_ascii_alphabetic())
        && !matches!(
            *tld,
            "home" | "internal" | "invalid" | "lan" | "local" | "test"
        )
}

fn normalize_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory(bytes).context("decoding favicon image")?;
    let image = image.thumbnail(64, 64);
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .context("encoding favicon PNG")?;
    Ok(png.into_inner())
}

fn save_icon(paths: &RsclipPaths, domain: &str, png: &[u8]) -> Result<()> {
    fs::create_dir_all(&paths.favicon_icon_dir)
        .with_context(|| format!("creating {}", paths.favicon_icon_dir.display()))?;
    let path = rsclip_core::favicons::icon_path(paths, domain);
    atomic_write(&path, png)
}

fn write_miss(paths: &RsclipPaths, domain: &str) -> Result<()> {
    fs::create_dir_all(&paths.favicon_miss_dir)
        .with_context(|| format!("creating {}", paths.favicon_miss_dir.display()))?;
    let path = rsclip_core::favicons::miss_path(paths, domain);
    atomic_write(&path, b"miss\n")
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp_path, contents).with_context(|| format!("writing {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| format!("renaming {}", path.display()))?;
    Ok(())
}

pub(crate) fn read_queue_domain(path: &Path) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    value
        .get("domain")
        .and_then(|value| value.as_str())
        .filter(|domain| !domain.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("queue file {} has no domain", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_paths(name: &str) -> RsclipPaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rsclip-daemon-favicon-test-{name}-{}-{unique}",
            std::process::id()
        ));
        RsclipPaths {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            data_dir: root.join("data"),
            db_path: root.join("state").join("rsclip.db"),
            image_dir: root.join("data").join("images"),
            thumb_dir: root.join("data").join("thumbs"),
            ocr_dir: root.join("data").join("ocr"),
            favicon_dir: root.join("data").join("favicons"),
            favicon_icon_dir: root.join("data").join("favicons").join("icons"),
            favicon_queue_dir: root.join("data").join("favicons").join("queue"),
            favicon_miss_dir: root.join("data").join("favicons").join("misses"),
            log_path: root.join("state").join("rsclip.log"),
            socket_path: root.join("rsclip.sock"),
        }
    }

    fn png_bytes() -> Vec<u8> {
        let image = ImageBuffer::from_pixel(96, 96, Rgba([40_u8, 120, 200, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("test PNG should encode");
        bytes.into_inner()
    }

    fn start_server(
        routes: Vec<(&'static str, u16, Vec<u8>)>,
        max_requests: usize,
    ) -> (Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let routes = routes
            .into_iter()
            .map(|(path, status, body)| (path.to_string(), (status, body)))
            .collect::<HashMap<_, _>>();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming().take(max_requests) {
                let Ok(mut stream) = stream else {
                    break;
                };
                handle_stream(&mut stream, &routes, &thread_requests);
            }
        });

        (
            Url::parse(&format!("http://{addr}/")).expect("test origin should parse"),
            requests,
        )
    }

    fn handle_stream(
        stream: &mut TcpStream,
        routes: &HashMap<String, (u16, Vec<u8>)>,
        requests: &Arc<Mutex<Vec<String>>>,
    ) {
        let mut buf = [0_u8; 2048];
        let Ok(size) = stream.read(&mut buf) else {
            return;
        };
        let request = String::from_utf8_lossy(&buf[..size]);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_ascii_whitespace().nth(1))
            .unwrap_or("/")
            .to_string();
        requests.lock().unwrap().push(path.clone());

        let (status, body) = routes
            .get(&path)
            .cloned()
            .unwrap_or_else(|| (404, b"not found".to_vec()));
        let reason = if status == 200 { "OK" } else { "Not Found" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&body);
    }

    #[test]
    fn favicon_ico_success_saves_png() {
        let paths = test_paths("ico-success");
        paths.ensure().unwrap();
        let (origin, _) = start_server(vec![("/favicon.ico", 200, png_bytes())], 1);

        fetch_and_cache_from_origin(&paths, "example.test", &origin).unwrap();

        assert!(rsclip_core::favicons::icon_path(&paths, "example.test").exists());
        assert!(!rsclip_core::favicons::miss_path(&paths, "example.test").exists());
    }

    #[test]
    fn html_discovery_with_relative_icon_saves_png() {
        let paths = test_paths("html-relative");
        paths.ensure().unwrap();
        let html = br#"<html><head><link rel="icon" href="/icon.png"></head></html>"#.to_vec();
        let (origin, requests) = start_server(
            vec![
                ("/", 200, html),
                ("/favicon.ico", 404, b"not found".to_vec()),
                ("/icon.png", 200, png_bytes()),
            ],
            3,
        );

        fetch_and_cache_from_origin(&paths, "example.test", &origin).unwrap();

        assert!(rsclip_core::favicons::icon_path(&paths, "example.test").exists());
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            ["/favicon.ico", "/", "/icon.png"]
        );
    }

    #[test]
    fn oversized_image_response_creates_miss() {
        let paths = test_paths("oversized");
        paths.ensure().unwrap();
        let oversized = vec![0_u8; IMAGE_CAP_BYTES + 2];
        let (origin, _) = start_server(vec![("/favicon.ico", 200, oversized)], 2);

        assert!(fetch_and_cache_from_origin(&paths, "example.test", &origin).is_err());

        assert!(rsclip_core::favicons::miss_path(&paths, "example.test").exists());
        assert!(!rsclip_core::favicons::icon_path(&paths, "example.test").exists());
    }

    #[test]
    fn invalid_image_creates_miss() {
        let paths = test_paths("invalid");
        paths.ensure().unwrap();
        let (origin, _) = start_server(vec![("/favicon.ico", 200, b"not image".to_vec())], 2);

        assert!(fetch_and_cache_from_origin(&paths, "example.test", &origin).is_err());

        assert!(rsclip_core::favicons::miss_path(&paths, "example.test").exists());
    }

    #[test]
    fn unsupported_svg_only_page_creates_miss() {
        let paths = test_paths("svg-only");
        paths.ensure().unwrap();
        let html = br#"<html><head><link rel="icon" href="/icon.svg"></head></html>"#.to_vec();
        let (origin, _) = start_server(
            vec![
                ("/favicon.ico", 404, b"not found".to_vec()),
                ("/", 200, html),
            ],
            2,
        );

        assert!(fetch_and_cache_from_origin(&paths, "example.test", &origin).is_err());

        assert!(rsclip_core::favicons::miss_path(&paths, "example.test").exists());
    }

    #[test]
    fn rejects_non_public_favicon_domains() {
        assert!(!is_fetchable_domain("localhost"));
        assert!(!is_fetchable_domain("127.0.0.1"));
        assert!(!is_fetchable_domain("example.1"));
        assert!(!is_fetchable_domain("example.local"));
        assert!(!is_fetchable_domain("example.test"));
        assert!(is_fetchable_domain("example.com"));
    }
}
