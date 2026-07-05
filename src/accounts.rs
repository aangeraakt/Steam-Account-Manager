use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    Unknown,
    Valid,
    Invalid,
    GuardRequired,
    Checking,
}

impl AccountStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "Onbekend",
            Self::Valid => "Geldig",
            Self::Invalid => "Ongeldig",
            Self::GuardRequired => "Guard vereist",
            Self::Checking => "Controleren...",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamAccount {
    pub id: String,
    pub username: String,
    pub password: String,
    pub alias: String,
    pub notes: String,
    pub steam_id: Option<String>,
    pub persona_name: Option<String>,
    pub avatar_url: Option<String>,
    pub refresh_token: Option<String>,
    pub machine_token: Option<String>,
    pub shared_secret: Option<String>,
    pub identity_secret: Option<String>,
    pub email: Option<String>,
    pub status: AccountStatus,
    pub last_validated: Option<DateTime<Utc>>,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(skip)]
    pub username_lower: String,
    #[serde(skip)]
    pub alias_lower: String,
}

impl SteamAccount {
    pub fn new(username: String, password: String) -> Self {
        let username_lower = username.to_lowercase();
        Self {
            id: Uuid::new_v4().to_string(),
            username,
            password,
            alias: String::new(),
            notes: String::new(),
            steam_id: None,
            persona_name: None,
            avatar_url: None,
            refresh_token: None,
            machine_token: None,
            shared_secret: None,
            identity_secret: None,
            email: None,
            status: AccountStatus::Unknown,
            last_validated: None,
            last_login: None,
            created_at: Utc::now(),
            username_lower,
            alias_lower: String::new(),
        }
    }

    pub fn display_name(&self) -> &str {
        if !self.alias.is_empty() {
            &self.alias
        } else {
            &self.username
        }
    }

    pub fn sync_search_fields(&mut self) {
        self.username_lower = self.username.to_lowercase();
        self.alias_lower = self.alias.to_lowercase();
    }

    pub fn matches_filter(&self, filter_lower: &str) -> bool {
        if filter_lower.is_empty() {
            return true;
        }
        self.username_lower.contains(filter_lower)
            || self.alias_lower.contains(filter_lower)
            || self
                .persona_name
                .as_ref()
                .map(|n| n.to_lowercase().contains(filter_lower))
                .unwrap_or(false)
            || self
                .steam_id
                .as_ref()
                .map(|id| id.contains(filter_lower))
                .unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStore {
    pub accounts: Vec<SteamAccount>,
}

impl AccountStore {
    pub fn new() -> Self {
        Self {
            accounts: Vec::new(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&SteamAccount> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut SteamAccount> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    pub fn add(&mut self, account: SteamAccount) {
        self.accounts.push(account);
        self.sort();
    }

    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(pos) = self.accounts.iter().position(|a| a.id == id) {
            self.accounts.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn sort(&mut self) {
        self.accounts.sort_by(|a, b| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        });
    }
}

impl Default for AccountStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_add_get_remove_and_sort() {
        let mut store = AccountStore::new();
        let mut b = SteamAccount::new("bravo".into(), "p".into());
        b.sync_search_fields();
        let mut a = SteamAccount::new("alpha".into(), "p".into());
        a.sync_search_fields();
        store.add(b);
        store.add(a);
        assert_eq!(store.accounts[0].username, "alpha");
        let id = store.accounts[0].id.clone();
        assert!(store.get(&id).is_some());
        assert!(store.remove(&id));
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn matches_filter_on_alias_and_steam_id() {
        let mut account = SteamAccount::new("hidden".into(), "p".into());
        account.alias = "Visible".into();
        account.steam_id = Some("76561198000000000".into());
        account.sync_search_fields();
        assert!(account.matches_filter("visible"));
        assert!(account.matches_filter("7656119"));
        assert!(!account.matches_filter("missing"));
    }

    #[test]
    fn status_labels_are_dutch() {
        assert_eq!(AccountStatus::Valid.label(), "Geldig");
        assert_eq!(AccountStatus::GuardRequired.label(), "Guard vereist");
    }
}
