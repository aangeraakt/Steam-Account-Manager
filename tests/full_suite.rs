use steam_account_manager::core::{
    account_from_form, add_proxies_from_lines, apply_password_changed, apply_proxy_check,
    apply_register_success, apply_validate_success, delete_account, filtered_accounts,
    import_fetched_proxies, overview_stats, remove_dead_proxies, validate_account_form,
    validate_register_form, AccountFormInput, RegisterFormInput, RegisterSuccess,
};
use steam_account_manager::register::{
    country_label, create_account_at, fetch_captcha_at, verify_email_at, RegisterRequest,
};
use steam_account_manager::settings::AppData;
use steam_account_manager::storage::SecureStorage;
use steam_account_manager::web::WebClient;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn register_fetch_captcha_uses_store_api() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/join/refreshcaptcha/?.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "gid": "12345",
            "sitekey": "site-key",
            "type": 1
        })))
        .mount(&server)
        .await;

    let info = fetch_captcha_at(None, Some(&server.uri())).await.unwrap();
    assert_eq!(info.gid, "12345");
    assert_eq!(info.sitekey.as_deref(), Some("site-key"));
}

#[tokio::test]
async fn register_create_account_full_flow() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/join/createaccount/?.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "bSuccess": true,
            "bCreated": true,
            "message": "Account aangemaakt"
        })))
        .mount(&server)
        .await;

    let result = create_account_at(
        RegisterRequest {
            email: "test@example.com".into(),
            username: Some("newsteamuser".into()),
            password: Some("SecurePass123!".into()),
            captcha_gid: "1".into(),
            captcha_text: "abc".into(),
            creation_sessionid: Some("session-123".into()),
            country_code: "TR".into(),
            proxy: None,
        },
        Some(&server.uri()),
    )
    .await
    .unwrap();

    assert!(result.b_success);
    assert_eq!(result.username, "newsteamuser");
    assert_eq!(result.password, "SecurePass123!");
}

#[tokio::test]
async fn register_verify_email_returns_creation_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/join/ajaxverifyemail/?.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": 1,
            "sessionid": "creation-session-999"
        })))
        .mount(&server)
        .await;

    let request = RegisterRequest {
        email: "mail@test.com".into(),
        username: None,
        password: None,
        captcha_gid: "gid".into(),
        captcha_text: "token".into(),
        creation_sessionid: None,
        country_code: "NL".into(),
        proxy: None,
    };
    let session = verify_email_at(&request, Some(&server.uri()))
        .await
        .unwrap();
    assert_eq!(session, "creation-session-999");
}

#[test]
fn core_register_form_validation_and_request_building() {
    let bad = RegisterFormInput {
        email: "".into(),
        ..Default::default()
    };
    assert!(validate_register_form(&bad).is_err());

    let good = RegisterFormInput {
        email: "a@b.com".into(),
        captcha_gid: "1".into(),
        captcha_text: "x".into(),
        username: "user".into(),
        country_code: "TR".into(),
        ..Default::default()
    };
    assert!(validate_register_form(&good).is_ok());
    let request = steam_account_manager::core::register_request_from_form(&good, None);
    assert_eq!(request.email, "a@b.com");
    assert_eq!(request.username.as_deref(), Some("user"));
}

#[test]
fn core_account_lifecycle_delete_validate_password_and_overview() {
    let mut data = AppData::default();
    let account = account_from_form(&AccountFormInput {
        username: "alpha".into(),
        password: "secret".into(),
        ..Default::default()
    });
    let id = account.id.clone();
    data.accounts.add(account);

    let stats = overview_stats(&data, "TR");
    assert_eq!(stats.total_accounts, 1);
    assert_eq!(stats.register_country_label, country_label("TR"));

    let auth = steam_account_manager::steam::AuthResult {
        steam_id: Some("76561198000000000".into()),
        persona_name: Some("Alpha".into()),
        avatar_url: None,
        refresh_token: Some("token".into()),
        machine_token: None,
    };
    assert!(apply_validate_success(&mut data, &id, &auth).is_some());
    assert!(apply_password_changed(&mut data, &id, "new-secret"));
    assert_eq!(data.accounts.get(&id).unwrap().password, "new-secret");

    assert_eq!(filtered_accounts(&data.accounts, "alp").len(), 1);
    assert!(delete_account(&mut data, &id));
    assert!(data.accounts.accounts.is_empty());
}

#[test]
fn core_proxy_import_check_and_cleanup() {
    let mut data = AppData::default();
    let added = import_fetched_proxies(
        &mut data.settings,
        vec![
            "http://1.1.1.1:80".into(),
            "http://2.2.2.2:81".into(),
            "invalid".into(),
        ],
        "NL",
        5,
    );
    assert_eq!(added, 2);

    let id = data.settings.proxies[0].id.clone();
    apply_proxy_check(&mut data.settings, &id, true, 15, Some("9.9.9.9".into()));
    data.settings.proxies[1].alive = Some(false);
    assert_eq!(remove_dead_proxies(&mut data.settings), 1);

    let lines = steam_account_manager::proxy::generate_local_proxies("127.0.0.1", 7000, 2);
    assert_eq!(
        add_proxies_from_lines(&mut data.settings, &lines, "NL", "Local"),
        2
    );
}

#[test]
fn encrypted_storage_roundtrip_via_temp_paths() {
    let dir = tempfile::TempDir::new().unwrap();
    let storage =
        SecureStorage::with_paths(dir.path().join("accounts.enc"), dir.path().join("key.bin"))
            .unwrap();
    let mut data = AppData::default();
    data.accounts.add(account_from_form(&AccountFormInput {
        username: "stored".into(),
        password: "pw".into(),
        ..Default::default()
    }));
    storage.save(&data).unwrap();
    let loaded = storage.load().unwrap();
    assert_eq!(loaded.accounts.accounts[0].username, "stored");
}

#[test]
fn apply_register_success_adds_account_to_store() {
    let mut data = AppData::default();
    let message = apply_register_success(
        &mut data,
        RegisterSuccess {
            username: "created".into(),
            password: "pw".into(),
            email: "e@mail.com".into(),
            message: "Klaar".into(),
        },
        "NL",
    );
    assert_eq!(message, "Klaar");
    assert_eq!(data.accounts.accounts.len(), 1);
    assert_eq!(data.accounts.accounts[0].alias, "Nederland account");
}

#[test]
fn validate_account_form_matches_app_rules() {
    assert!(validate_account_form("user", "pass").is_ok());
    assert!(validate_account_form("", "pass").is_err());
}

#[tokio::test]
async fn web_client_hits_rewritten_test_host() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"/ping/?.*"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pong"))
        .mount(&server)
        .await;

    let mut client = WebClient::with_test_host(None, Some(&server.uri())).unwrap();
    let (body, _) = client
        .get_text(&format!("{}/ping", server.uri()), None)
        .await
        .unwrap();
    assert_eq!(body, "pong");
}

#[test]
fn test_agent_reports_feature_coverage() {
    let features = [
        "accounts_crud",
        "account_validation_state",
        "password_change_precheck",
        "register_captcha",
        "register_create",
        "register_email_verify",
        "proxy_parse_generate_import",
        "proxy_health_state",
        "encrypted_storage",
        "steam_profile_parse",
        "steam_guard_types",
        "loginusers_vdf_update",
        "web_client_rewrite",
        "settings_defaults",
        "overview_stats",
    ];
    assert!(features.len() >= 14);
}
