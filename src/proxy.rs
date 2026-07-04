use anyhow::{anyhow, Context, Result};
use reqwest::Proxy;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyEntry {
    pub id: String,
    pub raw: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub scheme: String,
    pub country: String,
    pub label: String,
    pub alive: Option<bool>,
    pub latency_ms: Option<u64>,
    pub external_ip: Option<String>,
}

impl ProxyEntry {
    pub fn display(&self) -> String {
        if self.label.is_empty() {
            format!("{}:{} ({})", self.host, self.port, self.country)
        } else {
            format!("{} — {}", self.label, self.country)
        }
    }

    pub fn to_url(&self) -> String {
        let auth = match (&self.username, &self.password) {
            (Some(u), Some(p)) => format!("{u}:{p}@"),
            _ => String::new(),
        };
        format!("{}://{}{}:{}", self.scheme, auth, self.host, self.port)
    }
}

pub fn parse_proxy_line(line: &str, country: &str, label: &str) -> Result<ProxyEntry> {
    let raw = line.trim();
    if raw.is_empty() {
        anyhow::bail!("Lege proxy regel");
    }
    let (scheme, rest) = if let Some(stripped) = raw.strip_prefix("http://") {
        ("http", stripped)
    } else if let Some(stripped) = raw.strip_prefix("https://") {
        ("https", stripped)
    } else if let Some(stripped) = raw.strip_prefix("socks5://") {
        ("socks5", stripped)
    } else {
        ("http", raw)
    };
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((auth, hp)) if hp.contains(':') => (Some(auth), hp),
        _ => (None, rest),
    };
    let (host, port_str) = hostport
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("Ongeldig proxy formaat: {raw}"))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| anyhow!("Ongeldige poort: {port_str}"))?;
    let (username, password) = if let Some(auth) = auth {
        let (u, p) = auth
            .split_once(':')
            .ok_or_else(|| anyhow!("Ongeldige proxy auth: {auth}"))?;
        (Some(u.to_string()), Some(p.to_string()))
    } else {
        (None, None)
    };
    Ok(ProxyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        raw: raw.to_string(),
        host: host.to_string(),
        port,
        username,
        password,
        scheme: scheme.to_string(),
        country: country.to_string(),
        label: label.to_string(),
        alive: None,
        latency_ms: None,
        external_ip: None,
    })
}

pub fn build_reqwest_proxy(entry: &ProxyEntry) -> Result<Proxy> {
    let url = entry.to_url();
    Proxy::all(&url).with_context(|| format!("Kon proxy niet configureren: {url}"))
}

pub async fn check_proxy(entry: &ProxyEntry) -> Result<(bool, u64, Option<String>)> {
    let proxy = build_reqwest_proxy(entry)?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(15))
        .build()
        .context("Kon HTTP client niet maken")?;
    let start = Instant::now();
    let response = client
        .get("https://api.ipify.org?format=json")
        .send()
        .await
        .context("Proxy check mislukt")?;
    let latency = start.elapsed().as_millis() as u64;
    if !response.status().is_success() {
        return Ok((false, latency, None));
    }
    #[derive(Deserialize)]
    struct IpResponse {
        ip: String,
    }
    let body: IpResponse = response
        .json()
        .await
        .context("Kon IP response niet lezen")?;
    Ok((true, latency, Some(body.ip)))
}

pub async fn fetch_public_proxies(country: &str, limit: usize) -> Result<Vec<String>> {
    let country = country.trim().to_uppercase();
    let url = if country.is_empty() {
        "https://api.proxyscrape.com/v2/?request=get&protocol=http&timeout=5000&format=text"
            .to_string()
    } else {
        format!(
            "https://api.proxyscrape.com/v2/?request=get&protocol=http&timeout=5000&country={country}&format=text"
        )
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Kon HTTP client niet maken")?;
    let text = client
        .get(&url)
        .send()
        .await
        .context("Kon proxies niet ophalen")?
        .text()
        .await
        .context("Kon proxy lijst niet lezen")?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.contains(':'))
        .take(limit)
        .map(|l| format!("http://{l}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_proxy_with_auth() {
        let entry = parse_proxy_line("http://user:pass@1.2.3.4:8080", "NL", "test").unwrap();
        assert_eq!(entry.scheme, "http");
        assert_eq!(entry.host, "1.2.3.4");
        assert_eq!(entry.port, 8080);
        assert_eq!(entry.username.as_deref(), Some("user"));
        assert_eq!(entry.password.as_deref(), Some("pass"));
        assert_eq!(entry.country, "NL");
    }

    #[test]
    fn parse_bare_host_port_defaults_to_http() {
        let entry = parse_proxy_line("10.0.0.1:3128", "TR", "x").unwrap();
        assert_eq!(entry.scheme, "http");
        assert_eq!(entry.to_url(), "http://10.0.0.1:3128");
    }

    #[test]
    fn parse_socks5_scheme() {
        let entry = parse_proxy_line("socks5://127.0.0.1:9050", "US", "").unwrap();
        assert_eq!(entry.scheme, "socks5");
    }

    #[test]
    fn reject_invalid_proxy_lines() {
        assert!(parse_proxy_line("", "NL", "").is_err());
        assert!(parse_proxy_line("nocolon", "NL", "").is_err());
    }

    #[test]
    fn generate_local_proxies_increments_ports() {
        let lines = generate_local_proxies("127.0.0.1", 9000, 3);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains(":9000"));
        assert!(lines[2].contains(":9002"));
    }

    #[test]
    fn display_uses_label_when_present() {
        let entry = parse_proxy_line("1.1.1.1:80", "DE", "Primary").unwrap();
        assert!(entry.display().contains("Primary"));
    }
}

pub fn generate_local_proxies(base_host: &str, start_port: u16, count: u16) -> Vec<String> {
    (0..count)
        .map(|i| format!("http://{base_host}:{}", start_port + i))
        .collect()
}
