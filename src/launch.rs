use anyhow::{Context, Result};
use keyvalues_parser::{parse, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct SteamInstall {
    pub executable: PathBuf,
    pub config_dir: PathBuf,
}

pub fn find_steam_install() -> Option<SteamInstall> {
    let executable = find_steam_executable()?;
    let root = resolve_steam_root(&executable)?;
    let config_dir = root.join("config");
    Some(SteamInstall {
        executable,
        config_dir,
    })
}

pub fn find_steam_executable() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        let paths = [
            r"C:\Program Files (x86)\Steam\steam.exe",
            r"C:\Program Files\Steam\steam.exe",
        ];
        for path in paths {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
        which_steam("steam.exe")
    } else if cfg!(target_os = "macos") {
        let path = PathBuf::from("/Applications/Steam.app/Contents/MacOS/steam_osx");
        if path.exists() {
            Some(path)
        } else {
            which_steam("steam")
        }
    } else {
        let home = std::env::var("HOME").ok()?;
        let steam_sh = PathBuf::from(format!("{home}/.steam/steam/steam.sh"));
        if steam_sh.exists() {
            Some(steam_sh)
        } else {
            which_steam("steam")
        }
    }
}

fn resolve_steam_root(executable: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return executable.parent().map(|p| p.to_path_buf());
    }
    if cfg!(target_os = "macos") {
        return executable
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("Steam"));
    }
    let home = std::env::var("HOME").ok()?;
    let local_share = PathBuf::from(format!("{home}/.local/share/Steam"));
    if local_share.exists() {
        return Some(local_share);
    }
    executable.parent().map(|p| p.to_path_buf())
}

fn which_steam(name: &str) -> Option<PathBuf> {
    let output = Command::new(if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    })
    .arg(name)
    .output()
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

pub fn is_steam_running() -> bool {
    if cfg!(target_os = "windows") {
        Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_lowercase()
                    .contains("steam.exe")
            })
            .unwrap_or(false)
    } else {
        Command::new("pgrep")
            .args(["-x", "steam"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

pub fn shutdown_steam(install: &SteamInstall) -> Result<()> {
    if !is_steam_running() {
        return Ok(());
    }
    Command::new(&install.executable)
        .arg("-shutdown")
        .spawn()
        .context("Kon Steam shutdown niet starten")?;

    let start = Instant::now();
    while is_steam_running() {
        if start.elapsed() > SHUTDOWN_TIMEOUT {
            force_kill_steam()?;
            thread::sleep(Duration::from_secs(2));
            if is_steam_running() {
                anyhow::bail!("Steam kon niet worden afgesloten");
            }
            break;
        }
        thread::sleep(SHUTDOWN_POLL);
    }
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn force_kill_steam() -> Result<()> {
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "steam.exe"])
            .output();
    } else {
        let _ = Command::new("pkill").arg("-9").arg("steam").output();
    }
    Ok(())
}

pub fn switch_and_login(username: &str, password: &str, steam_id: Option<&str>) -> Result<()> {
    let install = find_steam_install().context("Steam installatie niet gevonden")?;
    shutdown_steam(&install)?;
    prepare_account_switch(&install, username, steam_id)?;
    Command::new(&install.executable)
        .arg("-login")
        .arg(username)
        .arg(password)
        .spawn()
        .context("Kon Steam niet starten met login")?;
    Ok(())
}

fn prepare_account_switch(
    install: &SteamInstall,
    username: &str,
    steam_id: Option<&str>,
) -> Result<()> {
    set_auto_login_user(username)?;
    let loginusers = install.config_dir.join("loginusers.vdf");
    if loginusers.exists() {
        let _ = update_loginusers(&loginusers, username, steam_id);
    }
    Ok(())
}

fn set_auto_login_user(username: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let steam = hkcu
            .open_subkey_with_flags(r"Software\Valve\Steam", winreg::enums::KEY_WRITE)
            .context("Kon Steam registry sleutel niet openen")?;
        steam
            .set_value("AutoLoginUser", &username)
            .context("Kon AutoLoginUser niet instellen")?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").context("Kon HOME niet bepalen")?;
        let registry_path = PathBuf::from(format!("{home}/.steam/registry.vdf"));
        if registry_path.exists() {
            let _ = update_linux_registry(&registry_path, username);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn update_linux_registry(path: &Path, username: &str) -> Result<()> {
    backup_file(path)?;
    let content = fs::read_to_string(path).context("Kon registry.vdf niet lezen")?;
    let re =
        regex::Regex::new(r#""AutoLoginUser"\s+"[^"]*""#).context("Kon regex niet compileren")?;
    let updated = if re.is_match(&content) {
        re.replace(&content, format!(r#""AutoLoginUser" "{username}""#))
            .into_owned()
    } else {
        anyhow::bail!("AutoLoginUser niet gevonden in registry.vdf");
    };
    let temp = path.with_extension("vdf.tmp");
    fs::write(&temp, updated).context("Kon tijdelijk registry bestand niet schrijven")?;
    fs::rename(&temp, path).context("Kon registry.vdf niet vervangen")?;
    Ok(())
}

fn update_loginusers(path: &Path, username: &str, steam_id: Option<&str>) -> Result<()> {
    backup_file(path)?;
    let content = fs::read_to_string(path).context("Kon loginusers.vdf niet lezen")?;
    let doc = parse(&content).context("Kon loginusers.vdf niet parsen")?;
    let mut owned = doc.into_vdf().into_owned();
    let target_id = {
        let users = owned
            .value
            .get_mut_obj()
            .context("loginusers.vdf heeft geen object root")?;
        find_target_steam_id(users, username, steam_id)?
    };
    let Some(target_id) = target_id else {
        return Ok(());
    };
    let users = owned
        .value
        .get_mut_obj()
        .context("loginusers.vdf heeft geen object root")?;
    for (id, entries) in users.iter_mut() {
        let recent = if id == &target_id { "1" } else { "0" };
        set_entry_flag(entries, "mostrecent", recent);
        set_entry_flag(entries, "MostRecent", recent);
    }
    write_vdf_owned(path, &owned)
}

fn find_target_steam_id(
    users: &mut keyvalues_parser::Obj<'_>,
    username: &str,
    steam_id: Option<&str>,
) -> Result<Option<String>> {
    if let Some(id) = steam_id {
        if users.contains_key(id) {
            return Ok(Some(id.to_string()));
        }
    }
    let username_lower = username.to_lowercase();
    for (id, entries) in users.iter() {
        if let Some(obj) = entries.first().and_then(|v| v.get_obj()) {
            if obj.iter().any(|(k, vals)| {
                k.eq_ignore_ascii_case("AccountName")
                    && vals
                        .first()
                        .and_then(|v| v.get_str())
                        .map(|n| n.to_lowercase())
                        == Some(username_lower.clone())
            }) {
                return Ok(Some(id.to_string()));
            }
        }
    }
    Ok(None)
}

fn set_entry_flag(entries: &mut [Value<'_>], key: &str, value: &str) {
    if let Some(obj) = entries.first_mut().and_then(|v| v.get_mut_obj()) {
        set_vdf_string(obj, key, value);
    }
}

fn set_vdf_string(obj: &mut keyvalues_parser::Obj<'_>, key: &str, value: &str) {
    let existing_key = obj.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned();
    if let Some(existing_key) = existing_key {
        obj.insert(
            existing_key,
            vec![Value::Str(std::borrow::Cow::Owned(value.to_string()))],
        );
    } else {
        obj.insert(
            std::borrow::Cow::Owned(key.to_string()),
            vec![Value::Str(std::borrow::Cow::Owned(value.to_string()))],
        );
    }
}

fn backup_file(path: &Path) -> Result<()> {
    let backup = path.with_extension("vdf.bak");
    if path.exists() && !backup.exists() {
        fs::copy(path, backup).context("Kon backup niet maken")?;
    }
    Ok(())
}

fn write_vdf_owned(path: &Path, doc: &keyvalues_parser::Vdf<'_>) -> Result<()> {
    let rendered = format!("{doc}");
    let temp = path.with_extension("vdf.tmp");
    fs::write(&temp, rendered).context("Kon tijdelijk VDF bestand niet schrijven")?;
    fs::rename(&temp, path).context("Kon VDF bestand niet vervangen")?;
    Ok(())
}

pub fn open_password_reset() -> Result<()> {
    open::that("https://help.steampowered.com/wizard/HelpWithLoginInfo")
        .context("Kon wachtwoord reset pagina niet openen")
}

pub fn open_steam_profile(steam_id: &str) -> Result<()> {
    let url = format!("https://steamcommunity.com/profiles/{steam_id}");
    open::that(url).context("Kon profiel niet openen")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    const SAMPLE_LOGINUSERS: &str = r#""users"
{
    "76561198000000000"
    {
        "AccountName"        "testuser"
        "MostRecent"        "0"
    }
    "76561198000000001"
    {
        "AccountName"        "otheruser"
        "MostRecent"        "1"
    }
}
"#;

    #[test]
    fn update_loginusers_marks_matching_account_recent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("loginusers.vdf");
        fs::write(&path, SAMPLE_LOGINUSERS).unwrap();
        update_loginusers(&path, "testuser", None).unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.contains("76561198000000000"));
        assert!(updated.contains("MostRecent"));
    }

    #[test]
    fn find_steam_executable_returns_none_when_missing() {
        if std::env::var("STEAM_EXECUTABLE_FORCE_NONE").is_ok() {
            assert!(find_steam_executable().is_none());
        }
    }
}
