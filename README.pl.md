# X1 Strontium ⏱

**Zdecentralizowany atomowy oracle czasu dla blockchain X1**

X1 Strontium dostarcza certyfikowany czas UTC on-chain poprzez agregację pomiarów z 45+ serwerów NTP Stratum-1 na 4 kontynentach. Operatorzy walidatorów uruchamiają lekki daemon który wysyła timestampy konsensusu do smart kontraktu Anchor, budując odporną na manipulacje referencję czasu którą może odczytać każdy program X1.

[![Build](https://img.shields.io/github/actions/workflow/status/PioWin-clo/strontium/release.yml)](https://github.com/PioWin-clo/strontium/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Problem który rozwiązuje

`Clock::unix_timestamp` na X1 pochodzi z zegara systemowego lidera bloku — może dryfować o 14–60 sekund od prawdziwego czasu UTC. Strontium zapewnia certyfikowaną, zdecentalizowaną referencję czasu opartą na zegarach atomowych.

```
Bez Strontium:  kontrakt.czas = zegar lidera (może być ±60s)
Ze Strontium:   kontrakt.czas = mediana z 45 serwerów atomowych (±5ms)
```

---

## Instalacja

### Wymagania
- Serwer Linux x86_64 z aktywnym walidatorem X1
- Co najmniej 5 XNT w keypairze oracle

### Krok 1: Pobierz binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium-linux-x86_64 \
  -O /usr/local/bin/strontium
chmod +x /usr/local/bin/strontium
ln -sf /usr/local/bin/strontium /usr/local/bin/x1sr
```

### Krok 2: Generuj oracle keypair

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
```

### Krok 3: Doładuj oracle keypair

```bash
solana transfer $(solana-keygen pubkey ~/.config/strontium/oracle-keypair.json) 5 \
  --url https://rpc.mainnet.x1.xyz \
  --keypair <TWOJ_WALLET> \
  --allow-unfunded-recipient
```

### Krok 4: Zarejestruj oracle

```bash
x1sr config set vote_keypair ~/.config/solana/vote.json
x1sr register
```

### Krok 5: Uruchom jako usługę systemd

```bash
sudo x1sr install
```

---

## Komendy

| Komenda | Opis |
|---------|------|
| `x1sr start` | Uruchom daemon (tryb live) |
| `x1sr start --dry-run` | Tryb testowy (bez transakcji) |
| `x1sr stop` | Zatrzymaj daemon |
| `x1sr status` | Pokaż status i konsensus NTP |
| `x1sr sources` | Pokaż szczegóły serwerów NTP |
| `x1sr balance` | Saldo keypair oracle |
| `x1sr register` | Rejestracja oracle (jednorazowo) |
| `x1sr config show` | Pokaż konfigurację |
| `x1sr config set <klucz> <wartość>` | Ustaw wartość konfiguracji |
| `x1sr install` | Zainstaluj jako usługę systemd |

---

## Konfiguracja

```bash
x1sr config set interval 300          # Interwał wysyłania (sekundy)
x1sr config set alert_webhook <URL>   # Webhook Telegram/Discord/Slack
x1sr config set alert_balance 1.0     # Próg alertu salda (XNT)
x1sr config set tier_threshold 60     # Próg konsensusu cross-tier (ms)
x1sr config set dry_run false         # Tryb live (true = testowy)
```

---

## Jak działa

```
1. Discovery — odpytuje 45 serwerów NTP Stratum-1 równolegle
2. Filtracja — IQR outlier filter, dedup po IP
3. Cross-tier — wymagana zgoda min. 2 niezależnych tierów
4. Konsensus — mediana timestampów (±5ms precyzja)
5. Rotacja — slot-hash wybiera operatora dla każdego okna
6. TX — wysyła submit_time + Memo z danymi diagnostycznymi
```

### Memo na blockchain

Każda transakcja zawiera czytelny rekord:
```
strontium:v1:w=1234:ntp=20:29:51.8621:chain=20:29:37.0000:c=87:s=7:st=1
```

| Pole | Znaczenie |
|------|-----------|
| `w=` | Numer okna czasowego |
| `ntp=` | Czas NTP (atomowy) HH:MM:SS.mmmm |
| `chain=` | Czas blockchain HH:MM:SS.mmmm |
| `c=` | Confidence 0-100% |
| `s=` | Liczba aktywnych źródeł NTP |
| `st=` | Najlepszy stratum (1 = atomowy) |

---

## Źródła NTP

45 serwerów Stratum-1 na 5 kontynentach:

| Region | Przykłady |
|--------|-----------|
| Europa | tempus1.gum.gov.pl, ptbtime1.ptb.de, ntp.metas.ch |
| Ameryka | time.nist.gov, nist1-atl.ustiming.org |
| Azja | ntp.nict.jp, ntp.jst.mfeed.ad.jp |
| Pacyfik | ntp.nml.csiro.au |
| Global | time.google.com, time.cloudflare.com |

---

## Odczyt czasu w smart kontrakcie

```rust
use strontium::TimeReading;

let reading: TimeReading = strontium::cpi::read_time(
    ctx, 
    max_staleness_slots // np. 300 (5 minut)
)?;

let time_ms        = reading.timestamp_ms;    // Unix ms (UTC, atomowy)
let confidence_pct = reading.confidence_pct; // 0-100
let spread_ms      = reading.spread_ms;      // rozrzut źródeł NTP
let sources        = reading.sources_count;  // liczba walidatorów
let staleness      = reading.staleness_slots; // ile slotów temu zaktualizowano
```

---

## Oracle PDA

| Parametr | Wartość |
|----------|---------|
| Program ID | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| Oracle PDA | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| Explorer | [explorer.mainnet.x1.xyz](https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn) |

---

## Kalkulator kosztów

👉 **[x1strontium.github.io/calculator](https://piowin-clo.github.io/strontium)**

---

## Licencja

MIT © 2026 Piotr "Killer" Winkler
