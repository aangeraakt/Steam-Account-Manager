# Steam Account Manager

Simple & Lightweight - Steam Account Manager

Een Rust desktop applicatie met egui/eframe voor het beheren van meerdere Steam accounts.

## Features

### Accounts
- Toevoegen, bewerken, verwijderen en zoeken
- Automatische validatie via Steam API
- Inloggen met Steam client switch
- Steam Guard (e-mail, authenticator, mobiele bevestiging)
- Auto Guard codes via shared secret
- Versleutelde lokale opslag (AES-256-GCM)

### Wachtwoord tools
- Genereer sterke wachtwoorden
- Automatisch wachtwoord wijzigen via Steam Help wizard
- Auto Steam Guard / mobiele bevestiging met identity secret

### Account registratie
- Steam account aanmaken via store API
- Proxy per registratie (bijv. TR voor Turks account)
- Captcha ophalen en e-mail verificatie flow

### Proxy manager
- Proxies handmatig toevoegen
- Ophalen per land (proxyscrape)
- Lokaal genereren (host + poort range)
- Check alive + latency + IP

## Bouwen

```bash
cargo build --release
cargo run --release
```

## GUI

- Sidebar met tabs: Accounts, Wachtwoord, Account maken, Proxies
- Overzicht panel met statistieken
- Modale dialogen voor Guard en CRUD

## Disclaimer

Niet geaffilieerd met Valve Corporation. Gebruik in overeenstemming met de Steam Subscriber Agreement.
