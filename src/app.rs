use crate::accounts::{AccountStatus, SteamAccount};
use crate::core::{
    account_from_form, add_proxies_from_lines, add_proxy_from_input, apply_login_success,
    apply_password_changed, apply_proxy_check, apply_register_success, apply_validate_failure,
    apply_validate_success, delete_account, import_fetched_proxies, register_request_from_form,
    remove_dead_proxies, remove_proxy, selected_proxy, sync_settings_from_ui,
    update_account_from_form, validate_account_form, validate_register_form, AccountFormInput,
    RegisterFormInput, RegisterSuccess,
};
use crate::launch::{
    find_steam_executable, open_password_reset, open_steam_profile, switch_and_login,
};
use crate::password::{change_account_password, PasswordChangeRequest};
use crate::proxy::{check_proxy, fetch_public_proxies, ProxyEntry};
use crate::register::{country_label, create_account, fetch_captcha};
use crate::settings::AppData;
use crate::steam::{
    auth_channel, authenticate, generate_guard_code, guard_prompt_channel, run_auth,
    validate_refresh_token, AuthRequest, GuardType,
};
use crate::storage::SecureStorage;
use crate::web::generate_secure_password;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(PartialEq, Eq)]
enum AppPhase {
    Idle,
    Working,
}

struct LoginOutcome {
    auth: crate::steam::AuthResult,
    message: String,
    launch_error: Option<String>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum AppTab {
    Accounts,
    Password,
    Register,
    Proxies,
}

enum WorkerMsg {
    ValidateDone {
        id: String,
        result: Result<crate::steam::AuthResult, String>,
    },
    LoginDone {
        id: String,
        result: Result<LoginOutcome, String>,
    },
    GuardRequired {
        guard_type: GuardType,
        detail: Option<String>,
    },
    PasswordChanged {
        id: String,
        result: Result<String, String>,
    },
    RegisterDone(Result<crate::register::RegisterResult, String>),
    CaptchaLoaded(Result<crate::register::CaptchaInfo, String>),
    ProxiesFetched(Result<Vec<String>, String>),
    ProxyChecked {
        id: String,
        alive: bool,
        latency_ms: u64,
        ip: Option<String>,
    },
    ProxyCheckDone,
}

#[derive(Clone)]
enum Dialog {
    None,
    AddAccount,
    EditAccount(String),
    DeleteConfirm(String),
    GuardInput {
        guard_type: GuardType,
        detail: Option<String>,
    },
    AccountDetails(String),
}

struct AccountForm {
    username: String,
    password: String,
    alias: String,
    notes: String,
    shared_secret: String,
    identity_secret: String,
    email: String,
    machine_token: String,
    show_password: bool,
    error: Option<String>,
}

impl AccountForm {
    fn from_account(account: &SteamAccount) -> Self {
        Self {
            username: account.username.clone(),
            password: account.password.clone(),
            alias: account.alias.clone(),
            notes: account.notes.clone(),
            shared_secret: account.shared_secret.clone().unwrap_or_default(),
            identity_secret: account.identity_secret.clone().unwrap_or_default(),
            email: account.email.clone().unwrap_or_default(),
            machine_token: account.machine_token.clone().unwrap_or_default(),
            show_password: false,
            error: None,
        }
    }

    fn clear(&mut self) {
        *self = Self {
            username: String::new(),
            password: String::new(),
            alias: String::new(),
            notes: String::new(),
            shared_secret: String::new(),
            identity_secret: String::new(),
            email: String::new(),
            machine_token: String::new(),
            show_password: false,
            error: None,
        };
    }
}

pub struct SteamAccountManagerApp {
    phase: AppPhase,
    data: AppData,
    storage: SecureStorage,
    tab: AppTab,
    selected_id: Option<String>,
    selected_proxy_id: Option<String>,
    filter: String,
    filter_lower: String,
    filter_snapshot: String,
    status_message: String,
    error_message: Option<String>,
    dialog: Dialog,
    account_form: AccountForm,
    guard_code_input: String,
    generated_password: String,
    custom_new_password: String,
    register_email: String,
    register_username: String,
    register_password: String,
    register_captcha_gid: String,
    register_captcha_text: String,
    register_creation_session: String,
    register_country: String,
    proxy_input: String,
    proxy_country_fetch: String,
    proxy_host_template: String,
    proxy_start_port: u16,
    proxy_generate_count: u16,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    guard_tx: Option<Sender<String>>,
    pending_operation: Option<String>,
    steam_found: bool,
    data_dir: String,
    clipboard_text: Option<String>,
}

enum AccountAction {
    Select(String),
    CopyGuard(String),
    OpenProfile(String),
    ShowDetails(String),
}

impl SteamAccountManagerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let storage = SecureStorage::new().expect("Kon opslag niet initialiseren");
        let data_dir = storage.data_dir_display();
        let data = storage.load().unwrap_or_else(|e| {
            eprintln!("Kon data niet laden: {e}");
            AppData::default()
        });
        let account_count = data.accounts.accounts.len();
        let register_country = data.settings.register_country.clone();
        let proxy_fetch = data.settings.proxy_fetch_country.clone();
        let proxy_host = data.settings.proxy_host_template.clone();
        let proxy_port = data.settings.proxy_start_port;
        let (tx, rx) = mpsc::channel();
        Self {
            phase: AppPhase::Idle,
            data,
            storage,
            tab: AppTab::Accounts,
            selected_id: None,
            selected_proxy_id: None,
            filter: String::new(),
            filter_lower: String::new(),
            filter_snapshot: String::new(),
            status_message: format!("{account_count} account(s) geladen."),
            error_message: None,
            dialog: Dialog::None,
            account_form: AccountForm {
                username: String::new(),
                password: String::new(),
                alias: String::new(),
                notes: String::new(),
                shared_secret: String::new(),
                identity_secret: String::new(),
                email: String::new(),
                machine_token: String::new(),
                show_password: false,
                error: None,
            },
            guard_code_input: String::new(),
            generated_password: String::new(),
            custom_new_password: String::new(),
            register_email: String::new(),
            register_username: String::new(),
            register_password: String::new(),
            register_captcha_gid: String::new(),
            register_captcha_text: String::new(),
            register_creation_session: String::new(),
            register_country,
            proxy_input: String::new(),
            proxy_country_fetch: proxy_fetch,
            proxy_host_template: proxy_host,
            proxy_start_port: proxy_port,
            proxy_generate_count: 10,
            tx,
            rx,
            guard_tx: None,
            pending_operation: None,
            steam_found: find_steam_executable().is_some(),
            data_dir,
            clipboard_text: None,
        }
    }

    fn sync_filter_cache(&mut self) {
        if self.filter != self.filter_snapshot {
            self.filter_snapshot.clone_from(&self.filter);
            self.filter_lower = self.filter.to_lowercase();
        }
    }

    fn is_busy(&self) -> bool {
        self.phase == AppPhase::Working
    }

    fn save_data(&mut self) {
        sync_settings_from_ui(
            &mut self.data,
            &self.register_country,
            &self.proxy_country_fetch,
            &self.proxy_host_template,
            self.proxy_start_port,
        );
        if let Err(e) = self.storage.save(&self.data) {
            self.error_message = Some(format!("Opslaan mislukt: {e}"));
        }
    }

    fn persist(&mut self, ctx: &egui::Context) {
        self.save_data();
        self.status_message = format!(
            "{} account(s) opgeslagen.",
            self.data.accounts.accounts.len()
        );
        let _ = ctx;
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::ValidateDone { id, result } => {
                    self.phase = AppPhase::Idle;
                    self.pending_operation = None;
                    self.guard_tx = None;
                    match result {
                        Ok(auth) => {
                            self.status_message =
                                apply_validate_success(&mut self.data, &id, &auth)
                                    .unwrap_or_else(|| "Validatie voltooid.".into());
                            self.persist(ctx);
                        }
                        Err(e) => {
                            apply_validate_failure(&mut self.data, &id);
                            self.error_message = Some(e);
                            self.status_message = "Validatie mislukt.".into();
                            self.persist(ctx);
                        }
                    }
                }
                WorkerMsg::LoginDone { id, result } => {
                    self.phase = AppPhase::Idle;
                    self.pending_operation = None;
                    self.guard_tx = None;
                    self.dialog = Dialog::None;
                    match result {
                        Ok(login) => {
                            apply_login_success(&mut self.data, &id, &login.auth);
                            self.persist(ctx);
                            self.status_message = login.message;
                            if let Some(err) = login.launch_error {
                                self.error_message = Some(err);
                            }
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                            self.status_message = "Inloggen mislukt.".into();
                        }
                    }
                }
                WorkerMsg::GuardRequired { guard_type, detail } => {
                    self.dialog = Dialog::GuardInput { guard_type, detail };
                    self.guard_code_input.clear();
                }
                WorkerMsg::PasswordChanged { id, result } => {
                    self.phase = AppPhase::Idle;
                    self.pending_operation = None;
                    self.guard_tx = None;
                    self.dialog = Dialog::None;
                    match result {
                        Ok(new_password) => {
                            apply_password_changed(&mut self.data, &id, &new_password);
                            self.generated_password = new_password.clone();
                            self.persist(ctx);
                            self.status_message =
                                "Wachtwoord gewijzigd. Nieuw wachtwoord opgeslagen.".into();
                            self.clipboard_text = Some(new_password);
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                            self.status_message = "Wachtwoord wijzigen mislukt.".into();
                        }
                    }
                }
                WorkerMsg::RegisterDone(result) => {
                    self.phase = AppPhase::Idle;
                    match result {
                        Ok(reg) => {
                            if reg.b_success {
                                self.status_message = apply_register_success(
                                    &mut self.data,
                                    RegisterSuccess {
                                        username: reg.username,
                                        password: reg.password,
                                        email: reg.email,
                                        message: reg.message,
                                    },
                                    &self.register_country,
                                );
                                self.persist(ctx);
                                self.tab = AppTab::Accounts;
                            } else {
                                self.error_message = Some(reg.message);
                            }
                        }
                        Err(e) => self.error_message = Some(e),
                    }
                }
                WorkerMsg::CaptchaLoaded(result) => {
                    self.phase = AppPhase::Idle;
                    match result {
                        Ok(info) => {
                            self.register_captcha_gid = info.gid;
                            self.status_message = format!(
                                "Captcha geladen{}",
                                info.sitekey
                                    .map(|s| format!(" (sitekey: {s})"))
                                    .unwrap_or_default()
                            );
                        }
                        Err(e) => self.error_message = Some(e),
                    }
                }
                WorkerMsg::ProxyChecked {
                    id,
                    alive,
                    latency_ms,
                    ip,
                } => {
                    apply_proxy_check(&mut self.data.settings, &id, alive, latency_ms, ip);
                    self.persist(ctx);
                }
                WorkerMsg::ProxiesFetched(result) => {
                    self.phase = AppPhase::Idle;
                    match result {
                        Ok(lines) => {
                            let country = self.proxy_country_fetch.clone();
                            let added = import_fetched_proxies(
                                &mut self.data.settings,
                                lines,
                                &country,
                                25,
                            );
                            self.persist(ctx);
                            self.status_message = format!("{added} proxy(s) toegevoegd.");
                        }
                        Err(e) => self.error_message = Some(e),
                    }
                }
                WorkerMsg::ProxyCheckDone => {
                    self.phase = AppPhase::Idle;
                    self.status_message = "Proxy check voltooid.".into();
                }
            }
            ctx.request_repaint();
        }
    }

    fn start_validate(&mut self, id: String, ctx: &egui::Context) {
        let account = match self.data.accounts.get(&id) {
            Some(a) => a.clone(),
            None => return,
        };
        if let Some(acc) = self.data.accounts.get_mut(&id) {
            acc.status = AccountStatus::Checking;
        }
        self.phase = AppPhase::Working;
        self.pending_operation = Some(id.clone());
        self.error_message = None;
        self.status_message = format!("Account '{}' controleren...", account.display_name());

        let (guard_tx, guard_rx) = auth_channel();
        let (guard_notify_tx, guard_notify_rx) = guard_prompt_channel();
        self.guard_tx = Some(guard_tx);
        let tx = self.tx.clone();
        let ctx_notify = ctx.clone();

        thread::spawn(move || {
            while let Ok(prompt) = guard_notify_rx.recv() {
                let _ = tx.send(WorkerMsg::GuardRequired {
                    guard_type: prompt.guard_type,
                    detail: prompt.detail,
                });
                ctx_notify.request_repaint();
            }
        });

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        run_auth(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt.block_on(async {
                if let Some(ref token) = account.refresh_token {
                    if !token.is_empty() {
                        if let Ok(auth) = validate_refresh_token(token).await {
                            return Ok(auth);
                        }
                    }
                }
                authenticate(AuthRequest {
                    username: account.username.clone(),
                    password: account.password.clone(),
                    machine_token: account.machine_token.clone(),
                    shared_secret: account.shared_secret.clone(),
                    guard_code: None,
                    guard_rx: Some(guard_rx),
                    guard_notify: Some(guard_notify_tx),
                })
                .await
                .map_err(|e| e.to_string())
            });
            let _ = tx.send(WorkerMsg::ValidateDone { id, result });
            ctx.request_repaint();
        });
    }

    fn start_login(&mut self, id: String, ctx: &egui::Context) {
        let account = match self.data.accounts.get(&id) {
            Some(a) => a.clone(),
            None => return,
        };
        self.phase = AppPhase::Working;
        self.pending_operation = Some(id.clone());
        self.error_message = None;
        self.status_message = format!(
            "Inloggen als '{}' — Steam wordt afgesloten en opnieuw gestart...",
            account.display_name()
        );

        let (guard_tx, guard_rx) = auth_channel();
        let (guard_notify_tx, guard_notify_rx) = guard_prompt_channel();
        self.guard_tx = Some(guard_tx);
        let tx = self.tx.clone();
        let ctx_notify = ctx.clone();

        thread::spawn(move || {
            while let Ok(prompt) = guard_notify_rx.recv() {
                let _ = tx.send(WorkerMsg::GuardRequired {
                    guard_type: prompt.guard_type,
                    detail: prompt.detail,
                });
                ctx_notify.request_repaint();
            }
        });

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        run_auth(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt.block_on(async {
                authenticate(AuthRequest {
                    username: account.username.clone(),
                    password: account.password.clone(),
                    machine_token: account.machine_token.clone(),
                    shared_secret: account.shared_secret.clone(),
                    guard_code: None,
                    guard_rx: Some(guard_rx),
                    guard_notify: Some(guard_notify_tx),
                })
                .await
                .map_err(|e| e.to_string())
            });
            let result = match result {
                Ok(auth) => {
                    let username = account.username.clone();
                    let password = account.password.clone();
                    let steam_id = auth.steam_id.clone();
                    match switch_and_login(&username, &password, steam_id.as_deref()) {
                        Ok(()) => Ok(LoginOutcome {
                            message: format!("Account gewisseld en Steam gestart voor {username}."),
                            auth,
                            launch_error: None,
                        }),
                        Err(e) => Ok(LoginOutcome {
                            message: format!("Ingelogd als {username}."),
                            auth,
                            launch_error: Some(format!("Steam starten mislukt: {e}")),
                        }),
                    }
                }
                Err(e) => Err(e),
            };
            let _ = tx.send(WorkerMsg::LoginDone { id, result });
            ctx.request_repaint();
        });
    }

    fn submit_guard_code(&mut self) {
        if let Some(tx) = self.guard_tx.take() {
            let _ = tx.send(self.guard_code_input.trim().to_string());
            self.dialog = Dialog::None;
            self.guard_code_input.clear();
            self.status_message = "Steam Guard code verzonden...".into();
        }
    }

    fn add_account(&mut self, ctx: &egui::Context) {
        if let Err(msg) =
            validate_account_form(&self.account_form.username, &self.account_form.password)
        {
            self.account_form.error = Some(msg.into());
            return;
        }
        let input = AccountFormInput {
            username: self.account_form.username.clone(),
            password: self.account_form.password.clone(),
            alias: self.account_form.alias.clone(),
            notes: self.account_form.notes.clone(),
            shared_secret: self.account_form.shared_secret.clone(),
            identity_secret: self.account_form.identity_secret.clone(),
            email: self.account_form.email.clone(),
            machine_token: self.account_form.machine_token.clone(),
        };
        let account = account_from_form(&input);
        let id = account.id.clone();
        self.data.accounts.add(account);
        self.dialog = Dialog::None;
        self.account_form.clear();
        self.selected_id = Some(id.clone());
        self.persist(ctx);
        self.start_validate(id, ctx);
    }

    fn update_account(&mut self, id: &str, ctx: &egui::Context) {
        if let Err(msg) =
            validate_account_form(&self.account_form.username, &self.account_form.password)
        {
            self.account_form.error = Some(msg.into());
            return;
        }
        if let Some(account) = self.data.accounts.get_mut(id) {
            let input = AccountFormInput {
                username: self.account_form.username.clone(),
                password: self.account_form.password.clone(),
                alias: self.account_form.alias.clone(),
                notes: self.account_form.notes.clone(),
                shared_secret: self.account_form.shared_secret.clone(),
                identity_secret: self.account_form.identity_secret.clone(),
                email: self.account_form.email.clone(),
                machine_token: self.account_form.machine_token.clone(),
            };
            update_account_from_form(account, &input);
        }
        self.dialog = Dialog::None;
        self.account_form.clear();
        self.persist(ctx);
        self.start_validate(id.to_string(), ctx);
    }

    fn selected_proxy(&self) -> Option<ProxyEntry> {
        selected_proxy(&self.data.settings, self.selected_proxy_id.as_deref())
    }

    fn start_password_change(&mut self, id: String, ctx: &egui::Context) {
        let account = match self.data.accounts.get(&id) {
            Some(a) => a.clone(),
            None => return,
        };
        self.phase = AppPhase::Working;
        self.pending_operation = Some(id.clone());
        self.error_message = None;
        self.status_message = format!("Wachtwoord wijzigen voor '{}'...", account.display_name());
        let new_password = if self.custom_new_password.trim().is_empty() {
            None
        } else {
            Some(self.custom_new_password.trim().to_string())
        };
        let proxy = self.selected_proxy();
        let (guard_tx, guard_rx) = auth_channel();
        let (guard_notify_tx, guard_notify_rx) = guard_prompt_channel();
        self.guard_tx = Some(guard_tx);
        let tx = self.tx.clone();
        let ctx_notify = ctx.clone();
        thread::spawn(move || {
            while let Ok(prompt) = guard_notify_rx.recv() {
                let _ = tx.send(WorkerMsg::GuardRequired {
                    guard_type: prompt.guard_type,
                    detail: prompt.detail,
                });
                ctx_notify.request_repaint();
            }
        });
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        run_auth(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt.block_on(async {
                change_account_password(PasswordChangeRequest {
                    username: account.username.clone(),
                    password: account.password.clone(),
                    shared_secret: account.shared_secret.clone(),
                    identity_secret: account.identity_secret.clone(),
                    steam_id: account.steam_id.clone(),
                    machine_token: account.machine_token.clone(),
                    new_password,
                    proxy,
                    guard_rx: Some(guard_rx),
                    guard_notify: Some(guard_notify_tx),
                })
                .await
                .map_err(|e| e.to_string())
            });
            let _ = tx.send(WorkerMsg::PasswordChanged { id, result });
            ctx.request_repaint();
        });
    }

    fn start_register(&mut self, ctx: &egui::Context) {
        let form = RegisterFormInput {
            email: self.register_email.clone(),
            username: self.register_username.clone(),
            password: self.register_password.clone(),
            captcha_gid: self.register_captcha_gid.clone(),
            captcha_text: self.register_captcha_text.clone(),
            creation_session: self.register_creation_session.clone(),
            country_code: self.register_country.clone(),
        };
        if let Err(msg) = validate_register_form(&form) {
            self.error_message = Some(msg.into());
            return;
        }
        self.phase = AppPhase::Working;
        self.status_message = "Steam account aanmaken...".into();
        let request = register_request_from_form(&form, self.selected_proxy());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        run_auth(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt
                .block_on(create_account(request))
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMsg::RegisterDone(result));
            ctx.request_repaint();
        });
    }

    fn start_fetch_captcha(&mut self, ctx: &egui::Context) {
        self.phase = AppPhase::Working;
        let proxy = self.selected_proxy();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        run_auth(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt
                .block_on(fetch_captcha(proxy.as_ref()))
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkerMsg::CaptchaLoaded(result));
            ctx.request_repaint();
        });
    }

    fn start_fetch_proxies(&mut self, ctx: &egui::Context) {
        self.phase = AppPhase::Working;
        let country = self.proxy_country_fetch.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            let result = rt.block_on(async {
                fetch_public_proxies(&country, 25)
                    .await
                    .map_err(|e| e.to_string())
            });
            let _ = tx.send(WorkerMsg::ProxiesFetched(result));
            ctx.request_repaint();
        });
    }

    fn start_check_all_proxies(&mut self, ctx: &egui::Context) {
        let proxies: Vec<ProxyEntry> = self.data.settings.proxies.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        self.phase = AppPhase::Working;
        self.status_message = "Proxies controleren...".into();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async {
                for proxy in proxies {
                    let id = proxy.id.clone();
                    let result = check_proxy(&proxy).await;
                    let (alive, latency, ip) = result.unwrap_or((false, 0, None));
                    let _ = tx.send(WorkerMsg::ProxyChecked {
                        id,
                        alive,
                        latency_ms: latency,
                        ip,
                    });
                    ctx.request_repaint();
                }
            });
            let _ = tx.send(WorkerMsg::ProxyCheckDone);
            ctx.request_repaint();
        });
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Menu");
        ui.add_space(8.0);
        if ui
            .selectable_label(self.tab == AppTab::Accounts, "Accounts")
            .clicked()
        {
            self.tab = AppTab::Accounts;
        }
        if ui
            .selectable_label(self.tab == AppTab::Password, "Wachtwoord")
            .clicked()
        {
            self.tab = AppTab::Password;
        }
        if ui
            .selectable_label(self.tab == AppTab::Register, "Account maken")
            .clicked()
        {
            self.tab = AppTab::Register;
        }
        if ui
            .selectable_label(self.tab == AppTab::Proxies, "Proxies")
            .clicked()
        {
            self.tab = AppTab::Proxies;
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("Overzicht").strong());
        ui.add_space(4.0);
        let total = self.data.accounts.accounts.len();
        let valid = self
            .data
            .accounts
            .accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Valid)
            .count();
        let proxies = self.data.settings.proxies.len();
        let alive = self
            .data
            .settings
            .proxies
            .iter()
            .filter(|p| p.alive == Some(true))
            .count();
        ui.label(format!("Accounts: {total}"));
        ui.label(format!("Geldig: {valid}"));
        ui.label(format!("Proxies: {proxies}"));
        ui.label(format!("Actieve proxies: {alive}"));
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(format!(
                "Land registratie: {}",
                country_label(&self.register_country)
            ))
            .small()
            .color(egui::Color32::GRAY),
        );
    }

    fn render_password_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("Wachtwoord beheer").strong());
        ui.add_space(6.0);
        ui.label("Genereer een sterk wachtwoord en wijzig het automatisch via Steam (Steam Guard + identity secret vereist voor volledige automatisering).");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Genereer wachtwoord").clicked() {
                self.generated_password = generate_secure_password(16);
                self.clipboard_text = Some(self.generated_password.clone());
            }
            if !self.generated_password.is_empty() {
                ui.label(format!("Laatste: {}", self.generated_password));
            }
        });
        ui.label("Eigen nieuw wachtwoord (optioneel):");
        ui.text_edit_singleline(&mut self.custom_new_password);
        ui.add_space(8.0);
        if let Some(ref id) = self.selected_id.clone() {
            if ui
                .add_enabled(!self.is_busy(), egui::Button::new("Wachtwoord wijzigen"))
                .clicked()
            {
                self.start_password_change(id.clone(), ctx);
            }
        } else {
            ui.label("Selecteer eerst een account in het Accounts tabblad.");
        }
    }

    fn render_register_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("Nieuw Steam account").strong());
        ui.add_space(6.0);
        ui.label(format!(
            "Regio proxy: {} ({})",
            country_label(&self.register_country),
            self.register_country
        ));
        ui.horizontal(|ui| {
            ui.label("Land code:");
            ui.text_edit_singleline(&mut self.register_country);
        });
        ui.label("E-mail:");
        ui.text_edit_singleline(&mut self.register_email);
        ui.label("Gebruikersnaam (optioneel):");
        ui.text_edit_singleline(&mut self.register_username);
        ui.label("Wachtwoord (optioneel, anders random):");
        ui.text_edit_singleline(&mut self.register_password);
        ui.label("Captcha gid:");
        ui.text_edit_singleline(&mut self.register_captcha_gid);
        ui.label("Captcha code / token:");
        ui.text_edit_singleline(&mut self.register_captcha_text);
        ui.label("Creation session (na e-mail bevestiging):");
        ui.text_edit_singleline(&mut self.register_creation_session);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.is_busy(), egui::Button::new("Captcha ophalen"))
                .clicked()
            {
                self.start_fetch_captcha(ctx);
            }
            if ui
                .add_enabled(!self.is_busy(), egui::Button::new("Account aanmaken"))
                .clicked()
            {
                self.start_register(ctx);
            }
        });
        if let Some(proxy) = self.selected_proxy() {
            ui.label(format!("Proxy: {}", proxy.display()));
        } else {
            ui.colored_label(egui::Color32::GRAY, "Geen proxy geselecteerd");
        }
    }

    fn render_proxy_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("Proxy beheer").strong());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Land (fetch):");
            ui.text_edit_singleline(&mut self.proxy_country_fetch);
            if ui.button("Ophalen").clicked() {
                self.start_fetch_proxies(ctx);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Host sjabloon:");
            ui.text_edit_singleline(&mut self.proxy_host_template);
            ui.label("Start poort:");
            ui.add(egui::DragValue::new(&mut self.proxy_start_port).range(1..=65535));
            ui.label("Aantal:");
            ui.add(egui::DragValue::new(&mut self.proxy_generate_count).range(1..=100));
            if ui.button("Genereer").clicked() {
                let lines = crate::proxy::generate_local_proxies(
                    &self.proxy_host_template,
                    self.proxy_start_port,
                    self.proxy_generate_count,
                );
                add_proxies_from_lines(
                    &mut self.data.settings,
                    &lines,
                    &self.proxy_country_fetch,
                    "Lokaal",
                );
                self.persist(ctx);
            }
        });
        ui.label("Proxy toevoegen (host:port of http://user:pass@host:port):");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.proxy_input);
            if ui.button("Toevoegen").clicked() {
                match add_proxy_from_input(
                    &mut self.data.settings,
                    &self.proxy_input,
                    &self.proxy_country_fetch,
                    "Handmatig",
                ) {
                    Ok(_) => {
                        self.proxy_input.clear();
                        self.persist(ctx);
                    }
                    Err(e) => self.error_message = Some(e),
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Check alle proxies").clicked() {
                self.start_check_all_proxies(ctx);
            }
            if ui.button("Dode verwijderen").clicked() {
                remove_dead_proxies(&mut self.data.settings);
                self.persist(ctx);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("proxy_list")
            .show(ui, |ui| {
                let mut remove_id = None;
                for proxy in &self.data.settings.proxies {
                    let selected = self.selected_proxy_id.as_deref() == Some(&proxy.id);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(selected, "").clicked() {
                            self.selected_proxy_id = Some(proxy.id.clone());
                            self.data.settings.default_proxy_id = Some(proxy.id.clone());
                        }
                        let status = match proxy.alive {
                            Some(true) => egui::RichText::new("OK")
                                .color(egui::Color32::from_rgb(80, 200, 120)),
                            Some(false) => egui::RichText::new("Dood")
                                .color(egui::Color32::from_rgb(255, 100, 100)),
                            None => egui::RichText::new("?").color(egui::Color32::GRAY),
                        };
                        ui.label(status);
                        ui.label(proxy.display());
                        if let Some(ms) = proxy.latency_ms {
                            ui.label(format!("{ms} ms"));
                        }
                        if let Some(ref ip) = proxy.external_ip {
                            ui.label(ip);
                        }
                        if ui.small_button("Verwijder").clicked() {
                            remove_id = Some(proxy.id.clone());
                        }
                    });
                }
                if let Some(id) = remove_id {
                    remove_proxy(&mut self.data.settings, &id, &mut self.selected_proxy_id);
                    self.persist(ctx);
                }
            });
    }

    fn copy_guard_code(&mut self, id: &str) -> Option<String> {
        if let Some(account) = self.data.accounts.get(id) {
            if let Some(ref secret) = account.shared_secret {
                match generate_guard_code(secret) {
                    Ok(code) => {
                        self.status_message =
                            format!("Guard code gekopieerd voor '{}'.", account.display_name());
                        return Some(code);
                    }
                    Err(e) => self.error_message = Some(e.to_string()),
                }
            } else {
                self.error_message = Some("Geen shared secret opgeslagen voor dit account.".into());
            }
        }
        None
    }

    fn render_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Steam Account Manager");
            ui.label(
                egui::RichText::new(match self.tab {
                    AppTab::Accounts => "Accounts",
                    AppTab::Password => "Wachtwoord tools",
                    AppTab::Register => "Account registratie",
                    AppTab::Proxies => "Proxy manager",
                })
                .color(egui::Color32::from_rgb(120, 170, 255)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.steam_found {
                    ui.colored_label(egui::Color32::from_rgb(255, 180, 60), "Steam niet gevonden");
                }
            });
        });
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.is_busy(), egui::Button::new("Account toevoegen"))
                .clicked()
            {
                self.account_form.clear();
                self.dialog = Dialog::AddAccount;
            }

            if let Some(ref id) = self.selected_id.clone() {
                if ui
                    .add_enabled(!self.is_busy(), egui::Button::new("Inloggen"))
                    .clicked()
                {
                    let id = id.clone();
                    self.start_login(id, ui.ctx());
                }
                if ui
                    .add_enabled(!self.is_busy(), egui::Button::new("Valideren"))
                    .clicked()
                {
                    let id = id.clone();
                    self.start_validate(id, ui.ctx());
                }
                if ui
                    .add_enabled(!self.is_busy(), egui::Button::new("Wachtwoord"))
                    .clicked()
                {
                    self.tab = AppTab::Password;
                }
                if ui.button("Bewerken").clicked() {
                    if let Some(account) = self.data.accounts.get(id) {
                        self.account_form = AccountForm::from_account(account);
                        self.dialog = Dialog::EditAccount(id.clone());
                    }
                }
                if ui.button("Verwijderen").clicked() {
                    self.dialog = Dialog::DeleteConfirm(id.clone());
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Zoeken:");
            ui.text_edit_singleline(&mut self.filter);

            ui.separator();

            if ui.button("Wachtwoord reset").clicked() {
                if let Err(e) = open_password_reset() {
                    self.error_message = Some(e.to_string());
                }
            }

            if ui.button("Steam map").clicked() {
                if let Some(path) = find_steam_executable() {
                    self.status_message = format!("Steam: {}", path.display());
                }
            }
        });
    }

    fn render_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.is_busy() {
                ui.add(egui::Spinner::new());
            }
            ui.label(&self.status_message);
        });
        if let Some(err) = &self.error_message {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
        }
    }

    fn render_account_list(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("account_list")
            .show(ui, |ui| {
                if self.data.accounts.accounts.is_empty() {
                    ui.label("Nog geen accounts toegevoegd.");
                    ui.add_space(4.0);
                    ui.label("Klik op 'Account toevoegen' om te beginnen.");
                    return;
                }

                let mut visible = false;
                let mut actions = Vec::new();
                for account in &self.data.accounts.accounts {
                    if !account.matches_filter(&self.filter_lower) {
                        continue;
                    }
                    visible = true;
                    let selected = self.selected_id.as_deref() == Some(&account.id);
                    let id = account.id.clone();

                    ui.horizontal(|ui| {
                        let response = ui.selectable_label(selected, "");
                        if response.clicked() {
                            actions.push(AccountAction::Select(id.clone()));
                        }

                        let (color, label) = status_badge(&account.status);
                        ui.colored_label(color, label);

                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(account.display_name()).strong());
                            let mut details = vec![format!("Gebruiker: {}", account.username)];
                            if let Some(ref persona) = account.persona_name {
                                if persona != &account.username {
                                    details.push(format!("Profiel: {persona}"));
                                }
                            }
                            if let Some(ref sid) = account.steam_id {
                                details.push(format!("SteamID: {sid}"));
                            }
                            if let Some(ref validated) = account.last_validated {
                                details.push(format!(
                                    "Gecontroleerd: {}",
                                    validated.format("%d-%m-%Y %H:%M")
                                ));
                            }
                            ui.label(details.join("  ·  "));
                            if !account.notes.is_empty() {
                                ui.label(
                                    egui::RichText::new(&account.notes)
                                        .italics()
                                        .color(egui::Color32::GRAY),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if account.shared_secret.is_some()
                                && ui.small_button("Guard code").clicked()
                            {
                                actions.push(AccountAction::CopyGuard(id.clone()));
                            }
                            if account.steam_id.is_some() && ui.small_button("Profiel").clicked() {
                                actions.push(AccountAction::OpenProfile(id.clone()));
                            }
                            if ui.small_button("Details").clicked() {
                                actions.push(AccountAction::ShowDetails(id.clone()));
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                if !visible && !self.data.accounts.accounts.is_empty() {
                    ui.label("Geen accounts gevonden.");
                }

                for action in actions {
                    match action {
                        AccountAction::Select(id) => self.selected_id = Some(id),
                        AccountAction::CopyGuard(id) => {
                            if let Some(code) = self.copy_guard_code(&id) {
                                self.clipboard_text = Some(code);
                            }
                        }
                        AccountAction::OpenProfile(id) => {
                            if let Some(sid) =
                                self.data.accounts.get(&id).and_then(|a| a.steam_id.clone())
                            {
                                let _ = open_steam_profile(&sid);
                            }
                        }
                        AccountAction::ShowDetails(id) => {
                            self.dialog = Dialog::AccountDetails(id);
                        }
                    }
                }
            });
    }

    fn render_account_dialog(&mut self, ctx: &egui::Context, title: &str, edit_id: Option<&str>) {
        let mut open = true;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Gebruikersnaam:");
                ui.text_edit_singleline(&mut self.account_form.username);
                ui.label("Wachtwoord:");
                ui.horizontal(|ui| {
                    if self.account_form.show_password {
                        ui.text_edit_singleline(&mut self.account_form.password);
                    } else {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.account_form.password)
                                .password(true),
                        );
                    }
                    if ui
                        .button(if self.account_form.show_password {
                            "Verberg"
                        } else {
                            "Toon"
                        })
                        .clicked()
                    {
                        self.account_form.show_password = !self.account_form.show_password;
                    }
                });
                ui.label("Alias (optioneel):");
                ui.text_edit_singleline(&mut self.account_form.alias);
                ui.label("Notities (optioneel):");
                ui.text_edit_multiline(&mut self.account_form.notes);
                ui.collapsing("Geavanceerd", |ui| {
                    ui.label("E-mail:");
                    ui.text_edit_singleline(&mut self.account_form.email);
                    ui.label("Shared secret (auto Guard codes):");
                    ui.text_edit_singleline(&mut self.account_form.shared_secret);
                    ui.label("Identity secret (auto bevestigingen):");
                    ui.text_edit_singleline(&mut self.account_form.identity_secret);
                    ui.label("Machine token:");
                    ui.text_edit_singleline(&mut self.account_form.machine_token);
                });
                if let Some(err) = &self.account_form.error {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Account wordt automatisch gevalideerd na opslaan.")
                        .small()
                        .color(egui::Color32::GRAY),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.is_busy(), egui::Button::new("Opslaan"))
                        .clicked()
                    {
                        if let Some(id) = edit_id {
                            self.update_account(id, ctx);
                        } else {
                            self.add_account(ctx);
                        }
                    }
                    if ui.button("Annuleren").clicked() {
                        self.dialog = Dialog::None;
                        self.account_form.clear();
                    }
                });
            });
        if !open {
            self.dialog = Dialog::None;
            self.account_form.clear();
        }
    }

    fn render_delete_dialog(&mut self, ctx: &egui::Context, id: &str) {
        let name = self
            .data
            .accounts
            .get(id)
            .map(|a| a.display_name().to_string())
            .unwrap_or_else(|| "dit account".to_string());
        let mut open = true;
        egui::Window::new("Account verwijderen")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Weet je zeker dat je '{name}' wilt verwijderen?"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Verwijderen").clicked() {
                        delete_account(&mut self.data, id);
                        if self.selected_id.as_deref() == Some(id) {
                            self.selected_id = None;
                        }
                        self.dialog = Dialog::None;
                        self.persist(ctx);
                        self.status_message =
                            format!("{} account(s) over.", self.data.accounts.accounts.len());
                    }
                    if ui.button("Annuleren").clicked() {
                        self.dialog = Dialog::None;
                    }
                });
            });
        if !open {
            self.dialog = Dialog::None;
        }
    }

    fn render_guard_dialog(
        &mut self,
        ctx: &egui::Context,
        guard_type: &GuardType,
        detail: &Option<String>,
    ) {
        let mut open = true;
        egui::Window::new("Steam Guard")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("{} vereist", guard_type.label()));
                if let Some(d) = detail {
                    ui.label(d);
                }
                if guard_type.needs_input() {
                    ui.label("Voer de code in:");
                    ui.text_edit_singleline(&mut self.guard_code_input);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Bevestigen").clicked() {
                            self.submit_guard_code();
                        }
                        if ui.button("Annuleren").clicked() {
                            if let Some(tx) = self.guard_tx.take() {
                                let _ = tx.send(String::new());
                            }
                            self.dialog = Dialog::None;
                            self.phase = AppPhase::Idle;
                            self.pending_operation = None;
                            self.status_message = "Geannuleerd.".into();
                        }
                    });
                } else {
                    ui.label("Bevestig de login in de Steam mobiele app of via e-mail.");
                    ui.add_space(8.0);
                    if ui.button("Sluiten").clicked() {
                        self.dialog = Dialog::None;
                    }
                }
            });
        if !open {
            self.dialog = Dialog::None;
        }
    }

    fn render_details_dialog(&mut self, ctx: &egui::Context, id: &str) {
        let account = match self.data.accounts.get(id) {
            Some(a) => a.clone(),
            None => {
                self.dialog = Dialog::None;
                return;
            }
        };
        let mut open = true;
        egui::Window::new("Account details")
            .collapsible(false)
            .resizable(true)
            .default_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Gebruikersnaam: {}", account.username));
                if let Some(ref sid) = account.steam_id {
                    ui.label(format!("SteamID: {sid}"));
                }
                if let Some(ref persona) = account.persona_name {
                    ui.label(format!("Profielnaam: {persona}"));
                }
                ui.label(format!("Status: {}", account.status.label()));
                if let Some(ref validated) = account.last_validated {
                    ui.label(format!(
                        "Laatst gecontroleerd: {}",
                        validated.format("%d-%m-%Y %H:%M")
                    ));
                }
                if let Some(ref login) = account.last_login {
                    ui.label(format!(
                        "Laatst ingelogd: {}",
                        login.format("%d-%m-%Y %H:%M")
                    ));
                }
                ui.label(format!(
                    "Guard: {}",
                    if account.shared_secret.is_some() {
                        "Authenticator (shared secret)"
                    } else {
                        "Handmatig / e-mail"
                    }
                ));
                ui.label(format!(
                    "Refresh token: {}",
                    if account.refresh_token.is_some() {
                        "Opgeslagen"
                    } else {
                        "Niet beschikbaar"
                    }
                ));
                if !account.notes.is_empty() {
                    ui.separator();
                    ui.label("Notities:");
                    ui.label(&account.notes);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Sluiten").clicked() {
                        self.dialog = Dialog::None;
                    }
                });
            });
        if !open {
            self.dialog = Dialog::None;
        }
    }

    fn render_footer(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Data: {}", self.data_dir))
                    .small()
                    .color(egui::Color32::GRAY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} account(s) · {} proxy(s)",
                        self.data.accounts.accounts.len(),
                        self.data.settings.proxies.len()
                    ))
                    .small()
                    .color(egui::Color32::GRAY),
                );
            });
        });
    }
}

fn status_badge(status: &AccountStatus) -> (egui::Color32, &str) {
    match status {
        AccountStatus::Unknown => (egui::Color32::GRAY, "Onbekend"),
        AccountStatus::Valid => (egui::Color32::from_rgb(80, 200, 120), "Geldig"),
        AccountStatus::Invalid => (egui::Color32::from_rgb(255, 100, 100), "Ongeldig"),
        AccountStatus::GuardRequired => (egui::Color32::from_rgb(255, 180, 60), "Guard"),
        AccountStatus::Checking => (egui::Color32::from_rgb(80, 180, 255), "Controleren"),
    }
}

impl eframe::App for SteamAccountManagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_filter_cache();
        self.poll_worker(ctx);

        if let Some(text) = self.clipboard_text.take() {
            ctx.copy_text(text);
        }

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(180.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_header(ui);
            ui.separator();
            if self.tab == AppTab::Accounts {
                self.render_toolbar(ui);
                ui.separator();
            }
            self.render_status(ui);
            ui.separator();
            match self.tab {
                AppTab::Accounts => {
                    self.render_account_list(ui);
                }
                AppTab::Password => {
                    self.render_password_panel(ui, ctx);
                }
                AppTab::Register => {
                    self.render_register_panel(ui, ctx);
                }
                AppTab::Proxies => {
                    self.render_proxy_panel(ui, ctx);
                }
            }
            self.render_footer(ui);
        });

        match self.dialog.clone() {
            Dialog::AddAccount => self.render_account_dialog(ctx, "Account toevoegen", None),
            Dialog::EditAccount(ref id) => {
                let id = id.clone();
                self.render_account_dialog(ctx, "Account bewerken", Some(&id));
            }
            Dialog::DeleteConfirm(ref id) => {
                let id = id.clone();
                self.render_delete_dialog(ctx, &id);
            }
            Dialog::GuardInput {
                guard_type, detail, ..
            } => {
                self.render_guard_dialog(ctx, &guard_type, &detail);
            }
            Dialog::AccountDetails(ref id) => {
                let id = id.clone();
                self.render_details_dialog(ctx, &id);
            }
            Dialog::None => {}
        }
    }
}
