use crate::proxy::ProxyEntry;
use crate::steam::{authenticate, AuthRequest, GuardPrompt};
use crate::web::{generate_secure_password, parse_query_params, WebClient};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use steam_auth::rsa_encrypt_password;

#[derive(Debug)]
pub struct PasswordChangeRequest {
    pub username: String,
    pub password: String,
    pub shared_secret: Option<String>,
    pub identity_secret: Option<String>,
    pub steam_id: Option<String>,
    pub machine_token: Option<String>,
    pub new_password: Option<String>,
    pub proxy: Option<ProxyEntry>,
    pub guard_rx: Option<Receiver<String>>,
    pub guard_notify: Option<std::sync::mpsc::Sender<GuardPrompt>>,
}

#[derive(Debug, Clone, Deserialize)]
struct PasswordChangeParams {
    s: String,
    account: String,
    reset: String,
    issueid: String,
    lost: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct JsonStatus {
    success: Option<serde_json::Value>,
    errorMsg: Option<String>,
    available: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct RsaResponse {
    success: bool,
    publickey_mod: Option<String>,
    publickey_exp: Option<String>,
    timestamp: Option<String>,
    errorMsg: Option<String>,
}

pub async fn change_account_password(request: PasswordChangeRequest) -> Result<String> {
    let new_password = request
        .new_password
        .clone()
        .unwrap_or_else(|| generate_secure_password(16));
    if new_password == request.password {
        anyhow::bail!("Nieuw wachtwoord is gelijk aan het huidige wachtwoord");
    }

    let auth = authenticate(AuthRequest {
        username: request.username.clone(),
        password: request.password.clone(),
        machine_token: request.machine_token.clone(),
        shared_secret: request.shared_secret.clone(),
        guard_code: None,
        guard_rx: request.guard_rx,
        guard_notify: request.guard_notify,
    })
    .await
    .context("Kon niet inloggen voor wachtwoord wijziging")?;

    let mut web = WebClient::new(request.proxy.as_ref())?;
    let cookies = build_cookie_strings(&auth, &request.username)?;
    web.set_cookies(&cookies);
    web.ensure_session_id("help.steampowered.com");

    let params = fetch_password_change_params(&mut web).await?;
    let sessionid = web.session_id("help.steampowered.com");

    wizard_get(
        &mut web,
        "https://help.steampowered.com/en/wizard/HelpWithLoginInfoEnterCode",
        &[
            ("s", &params.s),
            ("account", &params.account),
            ("reset", &params.reset),
            ("lost", &params.lost),
            ("issueid", &params.issueid),
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("gamepad", "0"),
        ],
    )
    .await?;

    post_wizard::<serde_json::Value>(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxSendAccountRecoveryCode",
        &[
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("gamepad", "0"),
            ("s", &params.s),
            ("method", "8"),
            ("link", ""),
            ("n", "1"),
        ],
    )
    .await?;

    if let (Some(identity), Some(steam_id)) = (
        request.identity_secret.as_ref(),
        request.steam_id.as_ref().or(auth.steam_id.as_ref()),
    ) {
        let _ = try_auto_mobile_confirm(&mut web, identity, steam_id, &params.s).await;
    }

    poll_recovery_confirmation(&mut web, &params, &sessionid).await?;

    get_wizard_json::<serde_json::Value>(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxVerifyAccountRecoveryCode",
        &[
            ("code", ""),
            ("s", &params.s),
            ("reset", &params.reset),
            ("lost", &params.lost),
            ("method", "8"),
            ("issueid", &params.issueid),
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("gamepad", "0"),
        ],
    )
    .await?;

    post_wizard::<serde_json::Value>(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxAccountRecoveryGetNextStep",
        &[
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("s", &params.s),
            ("account", &params.account),
            ("reset", &params.reset),
            ("issueid", &params.issueid),
            ("lost", "2"),
        ],
    )
    .await?;

    let old_encrypted = encrypt_for_user(&mut web, &request.username, &request.password).await?;
    post_wizard::<serde_json::Value>(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxAccountRecoveryVerifyPassword/",
        &[
            ("sessionid", &sessionid),
            ("s", &params.s),
            ("lost", "2"),
            ("reset", "1"),
            ("password", &old_encrypted.encrypted),
            ("rsatimestamp", &old_encrypted.timestamp),
        ],
    )
    .await?;

    let available: JsonStatus = post_wizard(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxCheckPasswordAvailable/",
        &[
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("password", &new_password),
        ],
    )
    .await?;
    if available.available == Some(false) {
        anyhow::bail!("Nieuw wachtwoord wordt niet geaccepteerd door Steam");
    }

    let new_encrypted = encrypt_for_user(&mut web, &request.username, &new_password).await?;
    post_wizard::<serde_json::Value>(
        &mut web,
        "https://help.steampowered.com/en/wizard/AjaxAccountRecoveryChangePassword/",
        &[
            ("sessionid", &sessionid),
            ("wizard_ajax", "1"),
            ("s", &params.s),
            ("account", &params.account),
            ("password", &new_encrypted.encrypted),
            ("rsatimestamp", &new_encrypted.timestamp),
        ],
    )
    .await?;

    Ok(new_password)
}

struct EncryptedPass {
    encrypted: String,
    timestamp: String,
}

fn build_cookie_strings(auth: &crate::steam::AuthResult, username: &str) -> Result<Vec<String>> {
    let mut cookies = vec![
        "mobileClient=android".to_string(),
        "mobileClientVersion=777777 3.0.0".to_string(),
        "Steam_Language=english".to_string(),
    ];
    if let Some(ref sid) = auth.steam_id {
        if let Some(ref token) = auth.refresh_token {
            cookies.push(format!("steamLogin={sid}||{token}"));
            cookies.push(format!("steamLoginSecure={sid}||{token}"));
        }
    }
    let _ = username;
    Ok(cookies)
}

async fn fetch_password_change_params(web: &mut WebClient) -> Result<PasswordChangeParams> {
    let (_, final_url) = web
        .get_text(
            "https://help.steampowered.com/wizard/HelpChangePassword?redir=store/account/",
            Some("https://store.steampowered.com/"),
        )
        .await?;
    let query = parse_query_params(&final_url);
    if let (Some(s), Some(account), Some(reset), Some(issueid), Some(lost)) = (
        query.get("s"),
        query.get("account"),
        query.get("reset"),
        query.get("issueid"),
        query.get("lost"),
    ) {
        return Ok(PasswordChangeParams {
            s: s.clone(),
            account: account.clone(),
            reset: reset.clone(),
            issueid: issueid.clone(),
            lost: lost.clone(),
        });
    }
    anyhow::bail!("Kon wachtwoord wijziging parameters niet ophalen — is Steam Guard actief?")
}

async fn wizard_get(web: &mut WebClient, url: &str, params: &[(&str, &str)]) -> Result<()> {
    let mut full = url.to_string();
    if !params.is_empty() {
        full.push('?');
        full.push_str(
            &params
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    let _ = web
        .get_text(&full, Some("https://help.steampowered.com/"))
        .await?;
    Ok(())
}

async fn get_wizard_json<T: serde::de::DeserializeOwned>(
    web: &mut WebClient,
    url: &str,
    params: &[(&str, &str)],
) -> Result<T> {
    let mut full = url.to_string();
    if !params.is_empty() {
        full.push('?');
        full.push_str(
            &params
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    web.get_json(&full, Some("https://help.steampowered.com/"))
        .await
}

async fn post_wizard<T: serde::de::DeserializeOwned>(
    web: &mut WebClient,
    url: &str,
    form: &[(&str, &str)],
) -> Result<T> {
    let response: T = web
        .post_form_json(url, form, Some("https://help.steampowered.com/"))
        .await?;
    Ok(response)
}

async fn encrypt_for_user(
    web: &mut WebClient,
    username: &str,
    password: &str,
) -> Result<EncryptedPass> {
    let sessionid = web.session_id("help.steampowered.com");
    let rsa: RsaResponse = web
        .post_form_json(
            "https://help.steampowered.com/en/login/getrsakey/",
            &[("sessionid", &sessionid), ("username", username)],
            Some("https://help.steampowered.com/"),
        )
        .await?;
    if !rsa.success {
        anyhow::bail!(
            "RSA key ophalen mislukt: {}",
            rsa.errorMsg.unwrap_or_default()
        );
    }
    let mod_hex = rsa.publickey_mod.context("Geen RSA modulus")?;
    let exp_hex = rsa.publickey_exp.context("Geen RSA exponent")?;
    let timestamp = rsa.timestamp.unwrap_or_else(|| "0".to_string());
    let encrypted = rsa_encrypt_password(password, &mod_hex, &exp_hex)
        .map_err(|e| anyhow!("Kon wachtwoord niet versleutelen: {e}"))?;
    Ok(EncryptedPass {
        encrypted,
        timestamp,
    })
}

async fn poll_recovery_confirmation(
    web: &mut WebClient,
    params: &PasswordChangeParams,
    sessionid: &str,
) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(90) {
        let response: JsonStatus = web
            .post_form_json(
                "https://help.steampowered.com/en/wizard/AjaxPollAccountRecoveryConfirmation",
                &[
                    ("sessionid", sessionid),
                    ("wizard_ajax", "1"),
                    ("s", &params.s),
                    ("reset", &params.reset),
                    ("lost", &params.lost),
                    ("method", "8"),
                    ("issueid", &params.issueid),
                    ("gamepad", "0"),
                ],
                Some("https://help.steampowered.com/"),
            )
            .await?;
        if response.errorMsg.as_deref().is_some_and(|m| !m.is_empty()) {
            anyhow::bail!("{}", response.errorMsg.unwrap_or_default());
        }
        if response
            .success
            .as_ref()
            .map(|v| v.as_bool().unwrap_or(false))
            == Some(true)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    anyhow::bail!("Steam Guard bevestiging verlopen — bevestig in de Steam app")
}

async fn try_auto_mobile_confirm(
    web: &mut WebClient,
    identity_secret: &str,
    steam_id: &str,
    creator_id: &str,
) -> Result<()> {
    let secret = steam_totp::Secret::from_string(identity_secret)
        .map_err(|e| anyhow!("Ongeldige identity secret: {e}"))?;
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let key = steam_totp::generate_confirmation_key(&secret, time, "change_password")
        .map_err(|e| anyhow!("Kon confirmation key niet genereren: {e}"))?;
    let sessionid = web.session_id("steamcommunity.com");
    let _ = web
        .post_form_text(
            "https://steamcommunity.com/mobileconf/ajaxop",
            &[
                ("op", "allow"),
                ("p", creator_id),
                ("a", steam_id),
                ("k", &key),
                ("t", &time.to_string()),
                ("m", "react"),
                ("tag", "change_password"),
                ("sessionid", &sessionid),
            ],
            Some("https://steamcommunity.com/mobileconf/conf"),
        )
        .await?;
    Ok(())
}
