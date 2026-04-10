# ⚛️ X1 Strontium

**Atomowa dokładność czasu dla blockchaina.**

🇬🇧 [English](README.md) | 🇵🇱 Polski

X1 Strontium to zdecentralizowany oracle czasu NTP dla [blockchaina X1](https://x1.xyz). Dostarcza kryptograficznie potwierdzonych znaczników czasu UTC zapisanych on-chain, pozyskanych z rządowych zegarów atomowych — weryfikowanych przez samą sieć walidatorów.

---

## Problem

Każdy blockchain ma problem z czasem. Na Solana/X1 `Clock::unix_timestamp` jest raportowany przez lidera slotu — może być manipulowany o ±1-2 sekundy bez wykrycia. Dla większości transakcji to nie ma znaczenia. Ale dla:

- **Kontraktów vestingowych** — dokładny moment wypłaty
- **Aukcji subsekudowych** — kto wygrał?
- **Dowodów czasu między łańcuchami** — weryfikowalne między sieciami
- **Kontraktów SLA** — znaczniki czasu uznawane przez sąd

...czas raportowany przez lidera to poważna luka bezpieczeństwa.

## Rozwiązanie

Każdy walidator uruchamia lekkiego daemona Strontium obok Tachyona. Co 5 minut (domyślnie):

1. Odpytuje 5 atomowych serwerów NTP z 3+ kontynentów (PTB Niemcy, GUM Polska, NIST USA, NICT Japonia + NTP Pool)
2. Oblicza skorygowane o RTT przesunięcia i sprawdza rozpiętość (próg ±50ms)
3. Jeśli konsensus osiągnięty → zapisuje znacznik czasu on-chain
4. Jeśli źródła się nie zgadzają → **milczy** (cisza jako sygnał)

Program on-chain agreguje zgłoszenia przez medianę. Żeby zmanipulować wynik, trzeba by jednocześnie skompromitować laboratoria zegarów atomowych na 3 kontynentach.

---

## Architektura

```
Serwer walidatora                    Blockchain X1
┌─────────────────────┐            ┌──────────────────────┐
│  Walidator Tachyon  │            │                      │
│                     │            │  Oracle PDA          │
│  Daemon Strontium   │───submit──▶│  ┌────────────────┐  │
│  ┌───────────────┐  │            │  │ trusted_time   │  │
│  │ Konsensus NTP │  │            │  │ spread_ms      │  │
│  │ PTB Niemcy    │  │            │  │ confidence     │  │
│  │ GUM Polska    │  │            │  │ ring_buffer    │  │
│  │ NIST USA      │  │            │  │ [1440 wpisów]  │  │
│  │ NICT Japonia  │  │            │  └────────────────┘  │
│  │ NTP Pool      │  │            │                      │
│  └───────────────┘  │            └──────────────────────┘
└─────────────────────┘
```

**Kluczowe właściwości:**
- Zero zaufania: żaden podmiot nie może zmodyfikować mediany
- Cisza jako sygnał: wbudowana tolerancja na błędy bizantyjskie
- Permissionless: każdy aktywny walidator X1 może dołączyć

---

## Wymagania

- **System:** Ubuntu 22.04 LTS (binary wymaga GLIBC 2.35+)
- **Solana CLI:** zainstalowane i w PATH (musi działać `solana-keygen`)
- **Saldo XNT:** ~1 XNT na oracle keypair (rejestracja + ~138 dni zgłoszeń)
- **Sieć:** port 123/UDP otwarty wychodzący (NTP)

---

## Szybki start

### Krok 1 — Pobierz binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium
chmod +x strontium
./strontium help
```

### Krok 2 — Wygeneruj oracle keypair

> ⚠️ **Ważne:** To jest NOWY dedykowany keypair — NIE używaj swojego `identity.json` ani `vote.json`.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

Zapisz wyświetlony pubkey — w następnym kroku będziesz musiał go zasilić.

### Krok 3 — Zasil oracle keypair

Oracle keypair płaci za rejestrację i bieżące zgłoszenia (~0.216 XNT/miesiąc). Wyślij minimum **1 XNT**:

```bash
# Sprawdź adres oracle keypair
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json

# Wyślij 1 XNT ze swojego głównego portfela
solana transfer \
  <ORACLE_PUBKEY> \
  1 \
  --url https://rpc.mainnet.x1.xyz \
  --keypair <TWÓJ_PORTFEL> \
  --allow-unfunded-recipient
```

### Krok 4 — Rejestracja

> ⚠️ **Ważne:** `vote.json` to keypair vote account Twojego walidatora — znajduje się na serwerze w `~/.config/solana/vote.json`. To NIE jest klucz Ledgera.

```bash
./strontium register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Oczekiwany wynik:
```
✓ Registration successful!
  TX: <sygnatura>
  Explorer: https://explorer.mainnet.x1.xyz/tx/<sygnatura>
```

### Krok 5 — Uruchom daemona

**Tryb testowy (bez transakcji):**
```bash
./strontium start --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Tryb live (w tle):**
```bash
nohup ./strontium start \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

**Sprawdź czy działa:**
```bash
tail -f ~/strontium.log
```

Powinieneś widzieć `✅ submit OK — tx: ...` co 5 minut.

---

## Rozwiązywanie problemów

### `GLIBC_2.39 not found`
Twój system jest za stary. Strontium wymaga Ubuntu 22.04+ (GLIBC 2.35+). Ubuntu 20.04 nie jest obsługiwane.

### `AccountNotFound` podczas rejestracji
Oracle keypair nie ma XNT. Patrz Krok 3 — najpierw zasil keypair.

### `AccountNotSigner` podczas rejestracji
Upewnij się że używasz właściwej ścieżki `--vote-keypair`. Na większości walidatorów to `~/.config/solana/vote.json`.

### Daemon milczy przez wiele cykli
```bash
tail -20 ~/strontium.log
```
Jeśli spread lub confidence jest niski, daemon poprawnie milczy (ochrona przed błędami bizantyjskimi). Sprawdź łączność NTP serwera: `ntpdate -q pool.ntp.org`

---

## Autostart przy restarcie (systemd)

```bash
sudo x1sr install
```

Komenda automatycznie:
- Wykrywa nazwę użytkownika i ścieżkę do binarnego
- Sprawdza saldo oracle keypair
- Generuje plik `/etc/systemd/system/strontium.service`
- Tworzy alias `x1sr` w systemie
- Włącza i uruchamia serwis

---

## Interfejs CLI

```bash
x1sr start              # uruchom daemona
x1sr stop               # zatrzymaj daemona
x1sr status             # stan + konsensus NTP + saldo
x1sr sources            # szczegóły źródeł NTP
x1sr history            # ostatnie zgłoszenia
x1sr config show        # aktualna konfiguracja
x1sr config set <k> <v> # zmień parametr
x1sr register           # rejestracja walidatora
x1sr balance            # saldo keypair + prognoza
x1sr install            # zainstaluj jako serwis systemd
```

---

## Źródła NTP

| Źródło | Typ | Stratum | Lokalizacja |
|---|---|---|---|
| PTB Niemcy | Rządowy atomowy | 1 | Brunszwik, DE |
| GUM Polska | Rządowy atomowy | 1 | Warszawa, PL |
| SYRTE Francja | Rządowy atomowy | 1 | Paryż, FR |
| METAS Szwajcaria | Rządowy atomowy | 1 | Berno, CH |
| Netnod Szwecja | Rządowy atomowy | 1 | Sztokholm, SE |
| NIST USA | Rządowy atomowy | 1 | Boulder, CO |
| NICT Japonia | Rządowy atomowy | 1 | Tokio, JP |
| NTP Pool (global) | Społecznościowy | 2-3 | Globalny |
| Cloudflare | Komercyjny (NTS) | 3 | Globalny |
| Google | Komercyjny | 3 | Globalny |

Daemon wybiera 5 serwerów o najniższym RTT z co najmniej 2 kontynentów. Preferowane źródła stratum 1 (rządowe atomowe) i NTS (z uwierzytelnianiem kryptograficznym).

---

## Adresy On-Chain

| | Adres |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [Zobacz na X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Dokładność

| Aktywni walidatorzy | Dokładność |
|---|---|
| 1 | ±3-10ms |
| 5 | ±2-6ms |
| 10 | ±2-5ms |
| 50+ | ±1-4ms |

Fizyczny limit: opóźnienie sieci NTP (~1-5ms). Przyszłe ulepszenie: moduły GPS/PPS → ±50 nanosekund.

---

## Koszty operacyjne

| Per walidator | Koszt |
|---|---|
| Dziennie | ~0.0014 XNT |
| Miesięcznie | ~0.043 XNT (~$0.02 przy aktualnych cenach) |

Przy 10 walidatorach z rotacją koszt spada ~10x. Koszty skalują się z ceną XNT.

---

## Bezpieczeństwo

**Upgrade authority:** Aktualnie w rękach autora projektu (`EgFaM42nFeZYwDXzMZWNTmp5ojyL7UGP8xgdX1SBXYsb`). Przekazanie do community multisig planowane w miarę wzrostu sieci submitterów.

**Model zagrożeń:**
- Jeden walidator kłamie → eliminowany przez medianę (wymaga >50% do wpłynięcia)
- Atak MITM na NTP → cross-kontynentalny check wykrywa rozbieżność
- Spam zgłoszeń → wymagana ValidatorRegistration (dowód vote account)
- Kompromitacja keypair → wyrejestruj + zarejestruj ponownie; tylko ~0.22 XNT zagrożone

**Odpowiedzialne ujawnianie błędów:** Otwórz [GitHub Issue](https://github.com/PioWin-clo/strontium/issues) lub skontaktuj się przez Telegram grupy walidatorów X1.

---

## Opcjonalne: GPS/PPS (eksperymentalne, nieprzetestowane)

Jeśli Twój serwer ma moduł GPS (`/dev/pps0`), Strontium automatycznie użyje go jako źródła czasu tier-0 (±50ns).

⚠️ Ta funkcja nie była testowana produkcyjnie. Feedback mile widziany przez GitHub Issues.

Rekomendowany sprzęt: u-blox NEO-M8N (~$30 + kabel USB)

---

## Roadmapa

- [x] Podstawowy daemon konsensusu NTP
- [x] Agregacja on-chain medianą (zero_copy, ring buffer 1440 wpisów)
- [x] ValidatorRegistration (dowód vote account, TTL 90 dni)
- [x] CLI chrony-style (x1sr status/sources/config/install)
- [ ] Weryfikacja minimalnego stake przy rejestracji
- [ ] NTS (Network Time Security) — kryptograficzne uwierzytelnianie
- [ ] Obsługa GPS/PPS — dokładność submilisekundowa
- [ ] Rotacja operatorów (window_id)
- [ ] Integracja Alpenglow (warstwa atestacji czasu)
- [ ] Community multisig upgrade authority

---

## Zbudowane na X1

X1 Strontium to open-source infrastruktura dla ekosystemu X1. Używa [Anchor](https://anchor-lang.com) 0.31.1 na [Tachyon](https://x1.xyz) 2.2.20.

**Pomysł i architektura:** Piotr "Killer" Winkler (z doświadczeń synchronizacji NTP→GUM na Fantomie)

*"Strontium" — nazwany od pierwiastka używanego w najdokładniejszych optycznych zegarach atomowych świata, dokładniejszych niż cezowe UTC.*
