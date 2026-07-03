# Steam Account Manager

Simple & Lightweight - Steam Account Manager

Een Rust desktop applicatie met een eenvoudige GUI (egui/eframe) voor het beheren van meerdere Steam accounts. Gebouwd in dezelfde stijl als [Driver-Updater](https://github.com/aangeraakt/Driver-Updater).

## Features

- Accounts toevoegen, bewerken en verwijderen
- Automatische validatie bij toevoegen/bewerken via Steam API
- Inloggen en Steam client starten
- Steam Guard ondersteuning (e-mail code, authenticator, mobiele bevestiging)
- Automatische Guard codes met shared secret
- Refresh token opslag voor snelle hervalidatie
- Wachtwoord reset via Steam help pagina
- Versleutelde lokale opslag (AES-256-GCM)
- Zoeken en filteren op accounts

## Vereisten

- Rust 1.85+ (via `rust-toolchain.toml`)
- Linux: `libgtk-3-dev`, `libxcb-render0-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`, `libxkbcommon-dev`
- Steam client geïnstalleerd (optioneel, voor direct inloggen)

## Bouwen

```bash
cargo build --release
```

## Uitvoeren

```bash
cargo run --release
```

## Gebruik

1. Klik op **Account toevoegen** en vul gebruikersnaam en wachtwoord in
2. Het account wordt automatisch gevalideerd tegen Steam
3. Bij Steam Guard verschijnt een prompt voor de verificatiecode
4. Selecteer een account en klik **Inloggen** om Steam te starten
5. Gebruik **Guard code** om een TOTP code te kopiëren (met shared secret)

Geavanceerde opties (shared secret, machine token) zijn beschikbaar onder het uitklapmenu bij toevoegen/bewerken.

## Data opslag

Accounts worden versleuteld opgeslagen in de applicatie data directory. De locatie wordt onderaan het venster getoond.

## Disclaimer

Deze applicatie is niet geaffilieerd met Valve Corporation. Gebruik op eigen risico en in overeenstemming met de [Steam Subscriber Agreement](https://store.steampowered.com/subscriber_agreement/).
