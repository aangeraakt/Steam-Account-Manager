use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use steam_auth::{
    CredentialsDetails, EAuthSessionGuardType, EAuthTokenPlatformType, LoginSession, SessionError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardType {
    EmailCode,
    DeviceCode,
    DeviceConfirmation,
    EmailConfirmation,
    Unknown,
}

impl GuardType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EmailCode => "E-mail code",
            Self::DeviceCode => "Authenticator code",
            Self::DeviceConfirmation => "Mobiele bevestiging",
            Self::EmailConfirmation => "E-mail bevestiging",
            Self::Unknown => "Steam Guard",
        }
    }

    pub fn needs_input(&self) -> bool {
        matches!(self, Self::EmailCode | Self::DeviceCode)
    }
}

impl From<EAuthSessionGuardType> for GuardType {
    fn from(value: EAuthSessionGuardType) -> Self {
        match value {
            EAuthSessionGuardType::KEAuthSessionGuardTypeEmailCode => Self::EmailCode,
            EAuthSessionGuardType::KEAuthSessionGuardTypeDeviceCode => Self::DeviceCode,
            EAuthSessionGuardType::KEAuthSessionGuardTypeDeviceConfirmation => {
                Self::DeviceConfirmation
            }
            EAuthSessionGuardType::KEAuthSessionGuardTypeEmailConfirmation => {
                Self::EmailConfirmation
            }
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GuardPrompt {
    pub guard_type: GuardType,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthResult {
    pub steam_id: Option<String>,
    pub persona_name: Option<String>,
    pub avatar_url: Option<String>,
    pub refresh_token: Option<String>,
    pub machine_token: Option<String>,
}

pub struct AuthRequest {
    pub username: String,
    pub password: String,
    pub machine_token: Option<String>,
    pub shared_secret: Option<String>,
    pub guard_code: Option<String>,
    pub guard_rx: Option<Receiver<String>>,
    pub guard_notify: Option<Sender<GuardPrompt>>,
}

pub fn generate_guard_code(shared_secret: &str) -> Result<String> {
    let secret = steam_totp::Secret::from_string(shared_secret)
        .map_err(|e| anyhow!("Ongeldige shared secret: {e}"))?;
    steam_totp::generate_auth_code(&secret, 0).map_err(|e| anyhow!("Kon code niet genereren: {e}"))
}

pub async fn authenticate(request: AuthRequest) -> Result<AuthResult> {
    let mut session = LoginSession::new(
        EAuthTokenPlatformType::KEAuthTokenPlatformTypeSteamClient,
        None,
    );

    let mut credentials = CredentialsDetails {
        account_name: request.username.clone(),
        password: request.password.clone(),
        persistence: None,
        steam_guard_machine_token: request.machine_token.clone(),
        steam_guard_code: request.guard_code.clone(),
    };

    if let Some(ref secret) = request.shared_secret {
        if credentials.steam_guard_code.is_none() {
            if let Ok(code) = generate_guard_code(secret) {
                credentials.steam_guard_code = Some(code);
            }
        }
    }

    let start_result = session
        .start_with_credentials(credentials)
        .await
        .map_err(map_session_error)?;

    if start_result.action_required {
        if let Some(actions) = &start_result.valid_actions {
            for action in actions {
                let guard_type = GuardType::from(action.guard_type);
                if guard_type == GuardType::DeviceCode {
                    if let Some(ref secret) = request.shared_secret {
                        let code = generate_guard_code(secret)?;
                        session
                            .submit_steam_guard_code(&code)
                            .await
                            .map_err(map_session_error)?;
                        continue;
                    }
                }
                if guard_type.needs_input() {
                    if let Some(ref notify) = request.guard_notify {
                        let _ = notify.send(GuardPrompt {
                            guard_type: guard_type.clone(),
                            detail: action.detail.clone(),
                        });
                    }
                    if let Some(ref rx) = request.guard_rx {
                        let code = rx
                            .recv()
                            .map_err(|_| anyhow!("Steam Guard invoer geannuleerd"))?;
                        if code.is_empty() {
                            anyhow::bail!("Steam Guard code is verplicht");
                        }
                        session
                            .submit_steam_guard_code(&code)
                            .await
                            .map_err(map_session_error)?;
                    } else {
                        anyhow::bail!("Steam Guard vereist: {}", guard_type.label());
                    }
                } else {
                    anyhow::bail!(
                        "Steam Guard vereist: {} — bevestig in de Steam app of via e-mail",
                        guard_type.label()
                    );
                }
            }
        }
    }

    let poll_interval = Duration::from_secs_f32(session.poll_interval());
    let timeout = Duration::from_secs(120);
    let start_time = Instant::now();

    let poll_result = loop {
        if start_time.elapsed() > timeout {
            anyhow::bail!("Inloggen verlopen (timeout)");
        }
        match session.poll().await {
            Ok(Some(result)) => break result,
            Ok(None) => {
                tokio::time::sleep(poll_interval).await;
            }
            Err(e) => return Err(map_session_error(e)),
        }
    };

    let steam_id = session.steam_id().map(|id| id.steam_id64().to_string());
    let refresh_token = Some(poll_result.refresh_token.clone());
    let machine_token = poll_result.new_guard_data.clone();

    let profile = if let Some(ref sid) = steam_id {
        fetch_profile(sid, &mut session).await.ok()
    } else {
        None
    };

    Ok(AuthResult {
        steam_id,
        persona_name: profile
            .as_ref()
            .map(|p| p.persona_name.clone())
            .or_else(|| Some(poll_result.account_name.clone())),
        avatar_url: profile.as_ref().and_then(|p| p.avatar_url.clone()),
        refresh_token,
        machine_token,
    })
}

pub async fn validate_refresh_token(token: &str) -> Result<AuthResult> {
    let mut session =
        LoginSession::from_refresh_token(token.to_string()).map_err(map_session_error)?;
    session
        .refresh_access_token()
        .await
        .map_err(map_session_error)?;

    let steam_id = session.steam_id().map(|id| id.steam_id64().to_string());
    let profile = if let Some(ref sid) = steam_id {
        fetch_profile(sid, &mut session).await.ok()
    } else {
        None
    };

    Ok(AuthResult {
        steam_id,
        persona_name: profile.as_ref().map(|p| p.persona_name.clone()),
        avatar_url: profile.as_ref().and_then(|p| p.avatar_url.clone()),
        refresh_token: session.refresh_token().map(|t| t.to_string()),
        machine_token: None,
    })
}

struct ProfileInfo {
    persona_name: String,
    avatar_url: Option<String>,
}

async fn fetch_profile(steam_id: &str, session: &mut LoginSession) -> Result<ProfileInfo> {
    let cookies = session.get_web_cookies().await.map_err(map_session_error)?;
    let cookie_header = cookies.join("; ");
    let client = reqwest::Client::new();
    let url = format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1");
    let response = client
        .get(&url)
        .header("Cookie", cookie_header)
        .send()
        .await
        .context("Kon profiel niet ophalen")?;
    let body = response.text().await.context("Kon profiel niet lezen")?;
    parse_profile_xml(&body)
}

fn parse_profile_xml(xml: &str) -> Result<ProfileInfo> {
    let persona_name = extract_xml_tag(xml, "steamID")
        .or_else(|| extract_xml_tag(xml, "customURL"))
        .unwrap_or_else(|| "Steam gebruiker".to_string());
    let avatar_url = extract_xml_tag(xml, "avatarFull");
    Ok(ProfileInfo {
        persona_name,
        avatar_url,
    })
}

fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn map_session_error(err: SessionError) -> anyhow::Error {
    let msg = err.to_string();
    if msg.to_lowercase().contains("invalid")
        || msg.to_lowercase().contains("credential")
        || msg.to_lowercase().contains("password")
    {
        anyhow!("Ongeldige inloggegevens")
    } else if msg.to_lowercase().contains("guard") {
        anyhow!("Steam Guard verificatie mislukt")
    } else {
        anyhow!("{msg}")
    }
}

pub fn run_auth<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::spawn(f);
}

pub fn auth_channel() -> (Sender<String>, Receiver<String>) {
    std::sync::mpsc::channel()
}

pub fn guard_prompt_channel() -> (Sender<GuardPrompt>, Receiver<GuardPrompt>) {
    std::sync::mpsc::channel()
}

pub fn apply_auth_result(
    account: &mut crate::accounts::SteamAccount,
    result: &AuthResult,
    mark_login: bool,
) {
    if let Some(ref sid) = result.steam_id {
        account.steam_id = Some(sid.clone());
    }
    if let Some(ref name) = result.persona_name {
        account.persona_name = Some(name.clone());
    }
    if let Some(ref url) = result.avatar_url {
        account.avatar_url = Some(url.clone());
    }
    if let Some(ref token) = result.refresh_token {
        account.refresh_token = Some(token.clone());
    }
    if let Some(ref token) = result.machine_token {
        account.machine_token = Some(token.clone());
    }
    account.status = crate::accounts::AccountStatus::Valid;
    account.last_validated = Some(Utc::now());
    if mark_login {
        account.last_login = Some(Utc::now());
    }
}

pub fn mark_invalid(account: &mut crate::accounts::SteamAccount) {
    account.status = crate::accounts::AccountStatus::Invalid;
    account.last_validated = Some(Utc::now());
}
