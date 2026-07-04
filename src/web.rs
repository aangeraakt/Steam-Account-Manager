use crate::proxy::ProxyEntry;
use anyhow::{Context, Result};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use reqwest::{Client, Url};
use std::collections::HashMap;

pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct WebClient {
    client: Client,
    cookies: HashMap<String, String>,
}

impl WebClient {
    pub fn new(proxy: Option<&ProxyEntry>) -> Result<Self> {
        let mut builder = Client::builder()
            .cookie_store(true)
            .user_agent(BROWSER_UA)
            .redirect(reqwest::redirect::Policy::limited(10));
        if let Some(entry) = proxy {
            builder = builder.proxy(crate::proxy::build_reqwest_proxy(entry)?);
        }
        Ok(Self {
            client: builder.build().context("Kon web client niet maken")?,
            cookies: HashMap::new(),
        })
    }

    pub fn set_cookies(&mut self, cookie_strings: &[String]) {
        for line in cookie_strings {
            if let Some((name, value)) = line.split_once('=') {
                self.cookies
                    .insert(name.trim().to_string(), value.trim().to_string());
            }
        }
    }

    pub fn session_id(&self, domain_hint: &str) -> String {
        self.cookies.get("sessionid").cloned().unwrap_or_else(|| {
            let sid = generate_session_id();
            let _ = domain_hint;
            sid
        })
    }

    pub fn ensure_session_id(&mut self, domain: &str) {
        if !self.cookies.contains_key("sessionid") {
            self.cookies
                .insert("sessionid".to_string(), generate_session_id());
        }
        let sid = self.cookies.get("sessionid").cloned().unwrap_or_default();
        self.cookies.insert(format!("sessionid@{domain}"), sid);
    }

    pub async fn get_text(&mut self, url: &str, referer: Option<&str>) -> Result<(String, Url)> {
        let headers = default_headers(referer);
        let req = self.client.get(url).headers(headers.clone());
        let req = self.apply_cookies(req, url);
        let response = req.send().await.context("GET request mislukt")?;
        let final_url = response.url().clone();
        self.store_response_cookies(&response);
        let text = response.text().await.context("Kon response niet lezen")?;
        Ok((text, final_url))
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &mut self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<T> {
        let mut headers = default_headers(referer);
        headers.insert(
            "X-Requested-With",
            HeaderValue::from_static("XMLHttpRequest"),
        );
        let req = self.client.get(url).headers(headers);
        let req = self.apply_cookies(req, url);
        let response = req.send().await.context("GET JSON mislukt")?;
        self.store_response_cookies(&response);
        response.json().await.context("Kon JSON niet parsen")
    }

    pub async fn post_form_json<T: serde::de::DeserializeOwned>(
        &mut self,
        url: &str,
        form: &[(&str, &str)],
        referer: Option<&str>,
    ) -> Result<T> {
        let mut headers = default_headers(referer);
        headers.insert(
            "X-Requested-With",
            HeaderValue::from_static("XMLHttpRequest"),
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded; charset=UTF-8"),
        );
        let req = self.client.post(url).headers(headers).form(form);
        let req = self.apply_cookies(req, url);
        let response = req.send().await.context("POST request mislukt")?;
        self.store_response_cookies(&response);
        response.json().await.context("Kon JSON niet parsen")
    }

    pub async fn post_form_text(
        &mut self,
        url: &str,
        form: &[(&str, &str)],
        referer: Option<&str>,
    ) -> Result<String> {
        let mut headers = default_headers(referer);
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded; charset=UTF-8"),
        );
        let req = self.client.post(url).headers(headers).form(form);
        let req = self.apply_cookies(req, url);
        let response = req.send().await.context("POST request mislukt")?;
        self.store_response_cookies(&response);
        response.text().await.context("Kon response niet lezen")
    }

    fn apply_cookies(&self, req: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
        if self.cookies.is_empty() {
            return req;
        }
        let cookie_header = self
            .cookies
            .iter()
            .filter(|(k, _)| !k.contains('@'))
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        if cookie_header.is_empty() {
            return req;
        }
        let _ = url;
        req.header("Cookie", cookie_header)
    }

    fn store_response_cookies(&mut self, response: &reqwest::Response) {
        for value in response.headers().get_all("set-cookie") {
            if let Ok(text) = value.to_str() {
                if let Some(part) = text.split(';').next() {
                    if let Some((name, val)) = part.split_once('=') {
                        self.cookies
                            .insert(name.trim().to_string(), val.trim().to_string());
                    }
                }
            }
        }
    }
}

fn default_headers(referer: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_UA));
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://help.steampowered.com"),
    );
    if let Some(r) = referer {
        if let Ok(v) = HeaderValue::from_str(r) {
            headers.insert(REFERER, v);
        }
    }
    headers
}

pub fn generate_session_id() -> String {
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

pub fn generate_secure_password(len: usize) -> String {
    const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGIT: &[u8] = b"23456789";
    const SPECIAL: &[u8] = b"!@#$%&*?_-";
    let mut rng = rand::thread_rng();
    let mut chars = vec![
        LOWER[rng.gen_range(0..LOWER.len())] as char,
        UPPER[rng.gen_range(0..UPPER.len())] as char,
        DIGIT[rng.gen_range(0..DIGIT.len())] as char,
        SPECIAL[rng.gen_range(0..SPECIAL.len())] as char,
    ];
    let all = [LOWER, UPPER, DIGIT, SPECIAL].concat();
    while chars.len() < len {
        chars.push(all[rng.gen_range(0..all.len())] as char);
    }
    chars.shuffle_manual(&mut rng);
    chars.into_iter().collect()
}

trait ShuffleChars {
    fn shuffle_manual(&mut self, rng: &mut impl Rng);
}

impl ShuffleChars for Vec<char> {
    fn shuffle_manual(&mut self, rng: &mut impl Rng) {
        for i in (1..self.len()).rev() {
            let j = rng.gen_range(0..=i);
            self.swap(i, j);
        }
    }
}

pub fn parse_query_params(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
