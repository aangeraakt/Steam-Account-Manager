use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

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

pub fn launch_steam(username: &str, password: Option<&str>) -> Result<()> {
    let steam = find_steam_executable().context("Steam installatie niet gevonden")?;
    let mut cmd = Command::new(&steam);
    cmd.arg("-login").arg(username);
    if let Some(pass) = password {
        cmd.arg(pass);
    }
    cmd.spawn().context("Kon Steam niet starten")?;
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
