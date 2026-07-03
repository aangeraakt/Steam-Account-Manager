use crate::accounts::{AccountStatus, AccountStore, SteamAccount};
use crate::launch::{find_steam_executable, launch_steam, open_password_reset, open_steam_profile};
use crate::steam::{
    apply_auth_result, auth_channel, authenticate, generate_guard_code, guard_prompt_channel,
    mark_invalid, run_auth, validate_refresh_token, AuthRequest, GuardType,
};
use crate::storage::SecureStorage;
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[derive(PartialEq, Eq)]
enum AppPhase {
    Idle,
    Working,
}

enum WorkerMsg {
    ValidateDone {
        id: String,
        result: Result<crate::steam::AuthResult, String>,
    },
    LoginDone {
        id: String,
        result: Result<crate::steam::AuthResult, String>,
    },
    GuardRequired {
        guard_type: GuardType,
        detail: Option<String>,
    },
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
            machine_token: String::new(),
            show_password: false,
            error: None,
        };
    }
}

pub struct SteamAccountManagerApp {
    phase: AppPhase,
    store: AccountStore,
    storage: SecureStorage,
    selected_id: Option<String>,
    filter: String,
    filter_lower: String,
    filter_snapshot: String,
    status_message: String,
    error_message: Option<String>,
    dialog: Dialog,
    account_form: AccountForm,
    guard_code_input: String,
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
        let store = storage.load().unwrap_or_else(|e| {
            eprintln!("Kon accounts niet laden: {e}");
            AccountStore::new()
        });
        let account_count = store.accounts.len();
        let (tx, rx) = mpsc::channel();
        Self {
            phase: AppPhase::Idle,
            store,
            storage,
            selected_id: None,
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
                machine_token: String::new(),
                show_password: false,
                error: None,
            },
            guard_code_input: String::new(),
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

    fn save_accounts(&mut self) {
        if let Err(e) = self.storage.save(&self.store) {
            self.error_message = Some(format!("Opslaan mislukt: {e}"));
        }
    }

    fn persist(&mut self, ctx: &egui::Context) {
        self.save_accounts();
        self.status_message = format!("{} account(s) opgeslagen.", self.store.accounts.len());
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
                            if let Some(account) = self.store.get_mut(&id) {
                                apply_auth_result(account, &auth, false);
                                self.status_message =
                                    format!("Account '{}' gevalideerd.", account.display_name());
                            }
                            self.persist(ctx);
                        }
                        Err(e) => {
                            if let Some(account) = self.store.get_mut(&id) {
                                mark_invalid(account);
                            }
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
                        Ok(auth) => {
                            let username = if let Some(account) = self.store.get_mut(&id) {
                                apply_auth_result(account, &auth, true);
                                account.username.clone()
                            } else {
                                String::new()
                            };
                            self.persist(ctx);
                            if !username.is_empty() {
                                match launch_steam(&username, None) {
                                    Ok(()) => {
                                        self.status_message =
                                            format!("Ingelogd en Steam gestart voor {username}.");
                                    }
                                    Err(e) => {
                                        self.status_message = format!("Ingelogd als {username}.");
                                        self.error_message =
                                            Some(format!("Steam starten mislukt: {e}"));
                                    }
                                }
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
            }
            ctx.request_repaint();
        }
    }

    fn start_validate(&mut self, id: String, ctx: &egui::Context) {
        let account = match self.store.get(&id) {
            Some(a) => a.clone(),
            None => return,
        };
        if let Some(acc) = self.store.get_mut(&id) {
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
        let account = match self.store.get(&id) {
            Some(a) => a.clone(),
            None => return,
        };
        self.phase = AppPhase::Working;
        self.pending_operation = Some(id.clone());
        self.error_message = None;
        self.status_message = format!("Inloggen als '{}'...", account.display_name());

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
        if self.account_form.username.trim().is_empty() {
            self.account_form.error = Some("Gebruikersnaam is verplicht.".into());
            return;
        }
        if self.account_form.password.is_empty() {
            self.account_form.error = Some("Wachtwoord is verplicht.".into());
            return;
        }
        let mut account = SteamAccount::new(
            self.account_form.username.trim().to_string(),
            self.account_form.password.clone(),
        );
        account.alias = self.account_form.alias.trim().to_string();
        account.notes = self.account_form.notes.trim().to_string();
        if !self.account_form.shared_secret.trim().is_empty() {
            account.shared_secret = Some(self.account_form.shared_secret.trim().to_string());
        }
        if !self.account_form.machine_token.trim().is_empty() {
            account.machine_token = Some(self.account_form.machine_token.trim().to_string());
        }
        account.sync_search_fields();
        let id = account.id.clone();
        self.store.add(account);
        self.dialog = Dialog::None;
        self.account_form.clear();
        self.selected_id = Some(id.clone());
        self.persist(ctx);
        self.start_validate(id, ctx);
    }

    fn update_account(&mut self, id: &str, ctx: &egui::Context) {
        if self.account_form.username.trim().is_empty() {
            self.account_form.error = Some("Gebruikersnaam is verplicht.".into());
            return;
        }
        if self.account_form.password.is_empty() {
            self.account_form.error = Some("Wachtwoord is verplicht.".into());
            return;
        }
        if let Some(account) = self.store.get_mut(id) {
            account.username = self.account_form.username.trim().to_string();
            account.password = self.account_form.password.clone();
            account.alias = self.account_form.alias.trim().to_string();
            account.notes = self.account_form.notes.trim().to_string();
            account.shared_secret = if self.account_form.shared_secret.trim().is_empty() {
                None
            } else {
                Some(self.account_form.shared_secret.trim().to_string())
            };
            account.machine_token = if self.account_form.machine_token.trim().is_empty() {
                None
            } else {
                Some(self.account_form.machine_token.trim().to_string())
            };
            account.sync_search_fields();
            account.status = AccountStatus::Unknown;
        }
        self.dialog = Dialog::None;
        self.account_form.clear();
        self.persist(ctx);
        self.start_validate(id.to_string(), ctx);
    }

    fn copy_guard_code(&mut self, id: &str) -> Option<String> {
        if let Some(account) = self.store.get(id) {
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
                if ui.button("Bewerken").clicked() {
                    if let Some(account) = self.store.get(id) {
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
                if self.store.accounts.is_empty() {
                    ui.label("Nog geen accounts toegevoegd.");
                    ui.add_space(4.0);
                    ui.label("Klik op 'Account toevoegen' om te beginnen.");
                    return;
                }

                let mut visible = false;
                let mut actions = Vec::new();
                for account in &self.store.accounts {
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
                            if account.steam_id.is_some()
                                && ui.small_button("Profiel").clicked()
                            {
                                actions.push(AccountAction::OpenProfile(id.clone()));
                            }
                            if ui.small_button("Details").clicked() {
                                actions.push(AccountAction::ShowDetails(id.clone()));
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                if !visible && !self.store.accounts.is_empty() {
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
                            if let Some(sid) = self.store.get(&id).and_then(|a| a.steam_id.clone())
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
                    ui.label("Shared secret (voor auto Guard codes):");
                    ui.text_edit_singleline(&mut self.account_form.shared_secret);
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
            .store
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
                        self.store.remove(id);
                        if self.selected_id.as_deref() == Some(id) {
                            self.selected_id = None;
                        }
                        self.dialog = Dialog::None;
                        self.persist(ctx);
                        self.status_message =
                            format!("{} account(s) over.", self.store.accounts.len());
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
        let account = match self.store.get(id) {
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
                    egui::RichText::new(format!("{} account(s)", self.store.accounts.len()))
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

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_header(ui);
            ui.separator();
            self.render_toolbar(ui);
            ui.separator();
            self.render_status(ui);
            ui.separator();
            self.render_account_list(ui);
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
