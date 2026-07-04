use crate::accounts::{AccountStatus, AccountStore, SteamAccount};
use crate::proxy::{parse_proxy_line, ProxyEntry};
use crate::register::{country_label, RegisterRequest};
use crate::settings::{AppData, AppSettings};
use crate::steam::{apply_auth_result, mark_invalid, AuthResult};

#[derive(Debug, Clone, Default)]
pub struct AccountFormInput {
    pub username: String,
    pub password: String,
    pub alias: String,
    pub notes: String,
    pub shared_secret: String,
    pub identity_secret: String,
    pub email: String,
    pub machine_token: String,
}

impl AccountFormInput {
    pub fn from_account(account: &SteamAccount) -> Self {
        Self {
            username: account.username.clone(),
            password: account.password.clone(),
            alias: account.alias.clone(),
            notes: account.notes.clone(),
            shared_secret: account.shared_secret.clone().unwrap_or_default(),
            identity_secret: account.identity_secret.clone().unwrap_or_default(),
            email: account.email.clone().unwrap_or_default(),
            machine_token: account.machine_token.clone().unwrap_or_default(),
        }
    }
}

pub fn validate_account_form(username: &str, password: &str) -> Result<(), &'static str> {
    if username.trim().is_empty() {
        return Err("Gebruikersnaam is verplicht.");
    }
    if password.is_empty() {
        return Err("Wachtwoord is verplicht.");
    }
    Ok(())
}

pub fn account_from_form(input: &AccountFormInput) -> SteamAccount {
    let mut account = SteamAccount::new(input.username.trim().to_string(), input.password.clone());
    account.alias = input.alias.trim().to_string();
    account.notes = input.notes.trim().to_string();
    if !input.shared_secret.trim().is_empty() {
        account.shared_secret = Some(input.shared_secret.trim().to_string());
    }
    if !input.identity_secret.trim().is_empty() {
        account.identity_secret = Some(input.identity_secret.trim().to_string());
    }
    if !input.email.trim().is_empty() {
        account.email = Some(input.email.trim().to_string());
    }
    if !input.machine_token.trim().is_empty() {
        account.machine_token = Some(input.machine_token.trim().to_string());
    }
    account.sync_search_fields();
    account
}

pub fn update_account_from_form(account: &mut SteamAccount, input: &AccountFormInput) {
    account.username = input.username.trim().to_string();
    account.password = input.password.clone();
    account.alias = input.alias.trim().to_string();
    account.notes = input.notes.trim().to_string();
    account.shared_secret = optional_trimmed(&input.shared_secret);
    account.identity_secret = optional_trimmed(&input.identity_secret);
    account.email = optional_trimmed(&input.email);
    account.machine_token = optional_trimmed(&input.machine_token);
    account.sync_search_fields();
    account.status = AccountStatus::Unknown;
}

fn optional_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegisterFormInput {
    pub email: String,
    pub username: String,
    pub password: String,
    pub captcha_gid: String,
    pub captcha_text: String,
    pub creation_session: String,
    pub country_code: String,
}

pub fn validate_register_form(input: &RegisterFormInput) -> Result<(), &'static str> {
    if input.email.trim().is_empty() {
        return Err("E-mail is verplicht.");
    }
    if input.captcha_gid.is_empty() || input.captcha_text.is_empty() {
        return Err("Captcha gid en code zijn verplicht.");
    }
    Ok(())
}

pub fn register_request_from_form(
    input: &RegisterFormInput,
    proxy: Option<ProxyEntry>,
) -> RegisterRequest {
    RegisterRequest {
        email: input.email.trim().to_string(),
        username: optional_trimmed(&input.username),
        password: if input.password.trim().is_empty() {
            None
        } else {
            Some(input.password.clone())
        },
        captcha_gid: input.captcha_gid.clone(),
        captcha_text: input.captcha_text.clone(),
        creation_sessionid: optional_trimmed(&input.creation_session),
        country_code: input.country_code.clone(),
        proxy,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewStats {
    pub total_accounts: usize,
    pub valid_accounts: usize,
    pub total_proxies: usize,
    pub alive_proxies: usize,
    pub register_country_label: String,
}

pub fn overview_stats(data: &AppData, register_country: &str) -> OverviewStats {
    OverviewStats {
        total_accounts: data.accounts.accounts.len(),
        valid_accounts: data
            .accounts
            .accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Valid)
            .count(),
        total_proxies: data.settings.proxies.len(),
        alive_proxies: data
            .settings
            .proxies
            .iter()
            .filter(|p| p.alive == Some(true))
            .count(),
        register_country_label: country_label(register_country).to_string(),
    }
}

pub fn filtered_accounts<'a>(store: &'a AccountStore, filter_lower: &str) -> Vec<&'a SteamAccount> {
    store
        .accounts
        .iter()
        .filter(|a| a.matches_filter(filter_lower))
        .collect()
}

pub fn add_proxies_from_lines(
    settings: &mut AppSettings,
    lines: &[String],
    country: &str,
    label: &str,
) -> usize {
    let mut added = 0;
    for line in lines {
        if let Ok(entry) = parse_proxy_line(line, country, label) {
            settings.proxies.push(entry);
            added += 1;
        }
    }
    added
}

pub fn add_proxy_from_input(
    settings: &mut AppSettings,
    input: &str,
    country: &str,
    label: &str,
) -> Result<ProxyEntry, String> {
    let entry = parse_proxy_line(input, country, label).map_err(|e| e.to_string())?;
    settings.proxies.push(entry.clone());
    Ok(entry)
}

pub fn remove_dead_proxies(settings: &mut AppSettings) -> usize {
    let before = settings.proxies.len();
    settings.proxies.retain(|p| p.alive != Some(false));
    before - settings.proxies.len()
}

pub fn remove_proxy(
    settings: &mut AppSettings,
    id: &str,
    selected_proxy_id: &mut Option<String>,
) -> bool {
    let before = settings.proxies.len();
    settings.proxies.retain(|p| p.id != id);
    if selected_proxy_id.as_deref() == Some(id) {
        *selected_proxy_id = None;
    }
    settings.proxies.len() < before
}

pub fn apply_proxy_check(
    settings: &mut AppSettings,
    id: &str,
    alive: bool,
    latency_ms: u64,
    ip: Option<String>,
) -> bool {
    if let Some(proxy) = settings.proxies.iter_mut().find(|p| p.id == id) {
        proxy.alive = Some(alive);
        proxy.latency_ms = Some(latency_ms);
        proxy.external_ip = ip;
        return true;
    }
    false
}

pub fn import_fetched_proxies(
    settings: &mut AppSettings,
    lines: Vec<String>,
    country: &str,
    limit: usize,
) -> usize {
    let mut added = 0;
    for line in lines.into_iter().take(limit) {
        if let Ok(entry) = parse_proxy_line(&line, country, "Fetched") {
            settings.proxies.push(entry);
            added += 1;
        }
    }
    added
}

pub fn selected_proxy(settings: &AppSettings, selected_id: Option<&str>) -> Option<ProxyEntry> {
    selected_id.and_then(|id| settings.proxies.iter().find(|p| p.id == id).cloned())
}

pub fn sync_settings_from_ui(
    data: &mut AppData,
    register_country: &str,
    proxy_fetch_country: &str,
    proxy_host_template: &str,
    proxy_start_port: u16,
) {
    data.settings.register_country = register_country.to_string();
    data.settings.proxy_fetch_country = proxy_fetch_country.to_string();
    data.settings.proxy_host_template = proxy_host_template.to_string();
    data.settings.proxy_start_port = proxy_start_port;
}

#[derive(Debug, Clone)]
pub struct RegisterSuccess {
    pub username: String,
    pub password: String,
    pub email: String,
    pub message: String,
}

pub fn apply_register_success(
    data: &mut AppData,
    reg: RegisterSuccess,
    register_country: &str,
) -> String {
    let mut account = SteamAccount::new(reg.username.clone(), reg.password.clone());
    account.email = Some(reg.email.clone());
    account.alias = format!("{} account", country_label(register_country));
    account.sync_search_fields();
    data.accounts.add(account);
    reg.message
}

pub fn apply_validate_success(data: &mut AppData, id: &str, auth: &AuthResult) -> Option<String> {
    let account = data.accounts.get_mut(id)?;
    let name = account.display_name().to_string();
    apply_auth_result(account, auth, false);
    Some(format!("Account '{name}' gevalideerd."))
}

pub fn apply_validate_failure(data: &mut AppData, id: &str) {
    if let Some(account) = data.accounts.get_mut(id) {
        mark_invalid(account);
    }
}

pub fn apply_login_success(data: &mut AppData, id: &str, auth: &AuthResult) -> Option<String> {
    let account = data.accounts.get_mut(id)?;
    let name = account.display_name().to_string();
    apply_auth_result(account, auth, true);
    Some(format!("Ingelogd als {name}."))
}

pub fn apply_password_changed(data: &mut AppData, id: &str, new_password: &str) -> bool {
    if let Some(account) = data.accounts.get_mut(id) {
        account.password = new_password.to_string();
        account.status = AccountStatus::Valid;
        return true;
    }
    false
}

pub fn delete_account(data: &mut AppData, id: &str) -> bool {
    data.accounts.remove(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::generate_local_proxies;

    #[test]
    fn validate_account_form_requires_fields() {
        assert!(validate_account_form("", "pass").is_err());
        assert!(validate_account_form("user", "").is_err());
        assert!(validate_account_form("user", "pass").is_ok());
    }

    #[test]
    fn account_from_form_maps_optional_fields() {
        let input = AccountFormInput {
            username: " steamuser ".into(),
            password: "secret".into(),
            alias: "Main".into(),
            shared_secret: "abc".into(),
            ..Default::default()
        };
        let account = account_from_form(&input);
        assert_eq!(account.username, "steamuser");
        assert_eq!(account.alias, "Main");
        assert_eq!(account.shared_secret.as_deref(), Some("abc"));
        assert!(account.identity_secret.is_none());
    }

    #[test]
    fn validate_register_form_checks_captcha() {
        let input = RegisterFormInput {
            email: "a@b.com".into(),
            captcha_gid: "1".into(),
            captcha_text: "code".into(),
            ..Default::default()
        };
        assert!(validate_register_form(&input).is_ok());
        let bad = RegisterFormInput {
            email: "a@b.com".into(),
            ..Default::default()
        };
        assert!(validate_register_form(&bad).is_err());
    }

    #[test]
    fn filtered_accounts_respects_search() {
        let mut store = AccountStore::new();
        let mut a = SteamAccount::new("alpha".into(), "p".into());
        a.sync_search_fields();
        store.add(a);
        assert_eq!(filtered_accounts(&store, "alp").len(), 1);
        assert_eq!(filtered_accounts(&store, "zzz").len(), 0);
    }

    #[test]
    fn proxy_management_add_remove_and_check() {
        let mut settings = AppSettings::default();
        let lines = generate_local_proxies("127.0.0.1", 9000, 3);
        assert_eq!(add_proxies_from_lines(&mut settings, &lines, "NL", "L"), 3);
        let id = settings.proxies[0].id.clone();
        assert!(apply_proxy_check(
            &mut settings,
            &id,
            true,
            42,
            Some("1.2.3.4".into())
        ));
        assert_eq!(settings.proxies[0].alive, Some(true));
        settings.proxies[1].alive = Some(false);
        assert_eq!(remove_dead_proxies(&mut settings), 1);
        let mut selected = Some(id.clone());
        assert!(remove_proxy(&mut settings, &id, &mut selected));
        assert!(selected.is_none());
    }

    #[test]
    fn apply_validate_and_login_update_account() {
        let mut data = AppData::default();
        let account = account_from_form(&AccountFormInput {
            username: "user".into(),
            password: "pass".into(),
            ..Default::default()
        });
        let id = account.id.clone();
        data.accounts.add(account);
        let auth = AuthResult {
            steam_id: Some("76561198000000000".into()),
            persona_name: Some("Tester".into()),
            avatar_url: None,
            refresh_token: Some("token".into()),
            machine_token: None,
        };
        assert!(apply_validate_success(&mut data, &id, &auth).is_some());
        assert_eq!(data.accounts.get(&id).unwrap().status, AccountStatus::Valid);
        apply_validate_failure(&mut data, &id);
        assert_eq!(
            data.accounts.get(&id).unwrap().status,
            AccountStatus::Invalid
        );
        assert!(apply_login_success(&mut data, &id, &auth).is_some());
        assert!(data.accounts.get(&id).unwrap().last_login.is_some());
    }

    #[test]
    fn apply_register_success_adds_account() {
        let mut data = AppData::default();
        let msg = apply_register_success(
            &mut data,
            RegisterSuccess {
                username: "newuser".into(),
                password: "newpass".into(),
                email: "e@mail.com".into(),
                message: "ok".into(),
            },
            "TR",
        );
        assert_eq!(msg, "ok");
        assert_eq!(data.accounts.accounts.len(), 1);
        assert_eq!(data.accounts.accounts[0].alias, "Turkije account");
    }
}
