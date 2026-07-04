use crate::proxy::ProxyEntry;
use crate::web::{generate_secure_password, WebClient};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct RegisterRequest {
    pub email: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub captcha_gid: String,
    pub captcha_text: String,
    pub creation_sessionid: Option<String>,
    #[allow(dead_code)]
    pub country_code: String,
    #[allow(dead_code)]
    pub proxy: Option<ProxyEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaptchaInfo {
    pub gid: String,
    pub sitekey: Option<String>,
    #[allow(dead_code)]
    pub r#type: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterResult {
    pub username: String,
    pub password: String,
    pub email: String,
    pub b_success: bool,
    pub message: String,
}

pub async fn fetch_captcha(proxy: Option<&ProxyEntry>) -> Result<CaptchaInfo> {
    fetch_captcha_at(proxy, None).await
}

pub async fn fetch_captcha_at(
    proxy: Option<&ProxyEntry>,
    api_host: Option<&str>,
) -> Result<CaptchaInfo> {
    let mut web = WebClient::with_test_host(proxy, api_host)?;
    web.ensure_session_id("store.steampowered.com");
    let info: CaptchaInfo = web
        .get_json(
            "https://store.steampowered.com/join/refreshcaptcha/?count=1&hcaptcha=1",
            Some("https://store.steampowered.com/join/"),
        )
        .await?;
    Ok(info)
}

pub async fn verify_email(request: &RegisterRequest) -> Result<String> {
    verify_email_at(request, None).await
}

pub async fn verify_email_at(request: &RegisterRequest, api_host: Option<&str>) -> Result<String> {
    let mut web = WebClient::with_test_host(request.proxy.as_ref(), api_host)?;
    web.ensure_session_id("store.steampowered.com");
    let sessionid = web.session_id("store.steampowered.com");
    #[derive(Deserialize)]
    struct VerifyResponse {
        success: Option<i32>,
        sessionid: Option<String>,
        message: Option<String>,
    }
    let response: VerifyResponse = web
        .post_form_json(
            "https://store.steampowered.com/join/ajaxverifyemail",
            &[
                ("email", &request.email),
                ("captchagid", &request.captcha_gid),
                ("captcha_text", &request.captcha_text),
                ("elang", "0"),
                ("init_id", "0"),
                ("guest", "0"),
                ("sessionid", &sessionid),
            ],
            Some("https://store.steampowered.com/join/"),
        )
        .await?;
    if response.success != Some(1) {
        anyhow::bail!(
            "E-mail verificatie mislukt: {}",
            response.message.unwrap_or_else(|| "Onbekende fout".into())
        );
    }
    response
        .sessionid
        .ok_or_else(|| anyhow!("Geen creation session ontvangen — bevestig eerst je e-mail"))
}

pub async fn create_account(request: RegisterRequest) -> Result<RegisterResult> {
    create_account_at(request, None).await
}

pub async fn create_account_at(
    request: RegisterRequest,
    api_host: Option<&str>,
) -> Result<RegisterResult> {
    let username = request.username.clone().unwrap_or_else(generate_username);
    let password = request
        .password
        .clone()
        .unwrap_or_else(|| generate_secure_password(14));
    let creation_sessionid = if let Some(id) = request.creation_sessionid.clone() {
        id
    } else {
        verify_email_at(&request, api_host).await?
    };

    let mut web = WebClient::with_test_host(request.proxy.as_ref(), api_host)?;
    web.ensure_session_id("store.steampowered.com");
    let sessionid = web.session_id("store.steampowered.com");

    #[derive(Deserialize)]
    #[allow(non_snake_case)]
    struct CreateResponse {
        bSuccess: Option<bool>,
        bCreated: Option<bool>,
        detail: Option<i32>,
        message: Option<String>,
    }

    let response: CreateResponse = web
        .post_form_json(
            "https://store.steampowered.com/join/createaccount/",
            &[
                ("accountname", &username),
                ("password", &password),
                ("count", "32"),
                ("lt", "0"),
                ("creation_sessionid", &creation_sessionid),
                ("sessionid", &sessionid),
                ("i_agree", "1"),
            ],
            Some("https://store.steampowered.com/join/"),
        )
        .await
        .context("Account aanmaken mislukt")?;

    let success = response.bSuccess.unwrap_or(false) || response.bCreated.unwrap_or(false);
    Ok(RegisterResult {
        username,
        password,
        email: request.email.clone(),
        b_success: success,
        message: response.message.unwrap_or_else(|| {
            if success {
                "Account aangemaakt".into()
            } else {
                format!("Aanmaken mislukt (detail={:?})", response.detail)
            }
        }),
    })
}

fn generate_username() -> String {
    let suffix: u32 = rand::random::<u32>() % 900_000 + 100_000;
    format!("steamuser{suffix}")
}

pub fn country_label(code: &str) -> &'static str {
    match code.to_uppercase().as_str() {
        "TR" => "Turkije",
        "NL" => "Nederland",
        "DE" => "Duitsland",
        "US" => "Verenigde Staten",
        "RU" => "Rusland",
        "FR" => "Frankrijk",
        _ => "Onbekend",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_label_maps_known_codes() {
        assert_eq!(country_label("TR"), "Turkije");
        assert_eq!(country_label("NL"), "Nederland");
        assert_eq!(country_label("XX"), "Onbekend");
    }
}
