# ⚛️ X1 Strontium

**Atomowa dokładność czasu dla blockchaina X1.**

[![CI](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml/badge.svg)](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml)
[![Built on X1](https://img.shields.io/badge/Built%20on-X1-black)](https://x1.xyz)

> Zielona odznaka CI oznacza że kod buduje się, przechodzi linting i czysty audyt bezpieczeństwa przy każdym commicie.

🇬🇧 [English](README.md) | 🇵🇱 Polski

X1 Strontium to zdecentralizowany oracle czasu NTP dla [blockchaina X1](https://x1.xyz). Dostarcza kryptograficznie poświadczone znaczniki czasu UTC on-chain, pozyskiwane z zegarów atomowych, komercyjnych serwerów NTP i pul społecznościowych — weryfikowanych przez samą sieć walidatorów.

---

## Problem

Na Solana/X1, `Clock::unix_timestamp` jest raportowany przez lidera bloku — może być manipulowany o ±1–2 sekundy bez wykrycia przez sieć. Dla większości transakcji jest to bez znaczenia. Ale dla:

- **Kontraktów vestingu** — dokładne terminy wypłat
- **Aukcji sub-sekundowych** — kto wygrał?
- **Dowodów czasu cross-chain** — weryfikowalnych między sieciami
- **Kontraktów SLA** — znaczniki czasu ważne prawnie

...czas raportowany przez lidera to poważna luka bezpieczeństwa. X1 Strontium to naprawia.

---

## Jak to działa

Każdy zarejestrowany walidator uruchamia lekki daemon Strontium obok Tachyon. Co **5 minut** (konfigurowalnie):

1. Odpytuje równolegle wszystkie 21 serwerów NTP — zegary atomowe, komercyjne, pule społecznościowe z 4 kontynentów
2. Wybiera 5 najlepszych źródeł według tieru (GPS/PPS → NTS → Stratum-1 → Pool) i RTT, deduplikując po IP
3. Oblicza skorygowaną o RTT medianę i weryfikuje spread (próg: ±50ms)
4. Waliduje konsensus między tierami — co najmniej 2 niezależne tiery muszą zgadzać się w oknie ±60ms
5. Oblicza **wynik pewności**: `liczba_źródeł × 0.4 + jakość_spreadu × 0.4 + waga_tieru × 0.2`
6. Sprawdza przesłany timestamp względem zegara on-chain — odrzuca jeśli odchylenie przekracza 10 sekund
7. Jeśli pewność ≥ 0.60 → przesyła timestamp on-chain przez `submit_time` + opcjonalny Memo Program
8. Jeśli źródła się nie zgadzają → **milczy** (cisza jako sygnał = ochrona przed błędami bizantyjskimi)

Każde zgłoszenie zawiera `sources_bitmap` — każda runda jest w pełni audytowalna on-chain. Program on-chain agreguje zgłoszenia w **buforze pierścieniowym 288 slotów** poprzez medianę ważoną stake.

---

## Wymagania

| Wymaganie | Szczegóły |
|---|---|
| **System** | Ubuntu 22.04 LTS lub nowszy (GLIBC 2.35+) |
| **Solana CLI** | Zainstalowane i w PATH (`solana-keygen` musi działać) |
| **Saldo XNT** | ≥1 XNT na oracle keypair |
| **Self-stake** | ≥100 XNT zweryfikowane na walidatorze |
| **Skip rate** | <10% (sprawdzane przy rejestracji) |
| **Sieć** | Port 123/UDP otwarty wychodzący (NTP) |
| **Status walidatora** | Aktywny na mainnecie |

> **Sprawdź port 123 UDP:**
> ```bash
> nc -zu pool.ntp.org 123 && echo "OK — port otwarty" || echo "ZABLOKOWANY — otwórz: sudo ufw allow out 123/udp"
> ```

---

## Szybki start

### Krok 1 — Pobierz binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium
chmod +x strontium
x1sr help
```

### Krok 2 — Wygeneruj oracle keypair

> ⚠️ **Tylko nowy, dedykowany keypair.** NIE używaj `identity.json` ani `vote.json`.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

### Krok 3 — Doładuj oracle keypair

```bash
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json
# Wyślij XNT na ten adres przez XDEX, Backpack, CLI lub Ledger
```

Użyj **[kalkulatora kosztów](https://piowin-clo.github.io/strontium)** żeby wybrać odpowiednią kwotę.

### Krok 4 — Zarejestruj się

```bash
x1sr register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Rejestracja weryfikuje: aktywny walidator, skip rate <10%, self-stake ≥100 XNT. Wygasa po **90 dniach**.

### Krok 5 — Uruchom daemon

**Tryb testowy** (zero kosztów):

```bash
x1sr start --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Tryb live:**

```bash
nohup x1sr start \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

```bash
x1sr status
tail -f ~/strontium.log   # Powinieneś zobaczyć: ✅ submit OK — tx: ...
```

### Krok 6 — Zainstaluj jako usługę systemową

```bash
sudo x1sr install
```

---

## Dokumentacja CLI

```
x1sr start              Uruchom daemon (tryb live)
x1sr start --dry-run    Uruchom w trybie testowym (bez transakcji)
x1sr stop               Zatrzymaj daemon
x1sr status             Status, konsensus NTP, saldo, rotacja
x1sr sources            Tabela źródeł NTP (RTT, offset, tier, NTS)
x1sr history [N]        Ostatnie N zgłoszeń on-chain (domyślnie: 10)
x1sr register           Zarejestruj oracle walidatora
x1sr deregister         Wyrejestruj (wkrótce)
x1sr balance            Saldo oracle keypair i runway
x1sr archive            Eksportuj historię on-chain do JSONL
x1sr config show        Pokaż aktualną konfigurację
x1sr config set K V     Ustaw wartość konfiguracji
x1sr install            Zainstaluj jako usługę systemd (uruchom z sudo)
x1sr uninstall          Usuń usługę systemd
```

**Klucze konfiguracji:**

| Klucz | Domyślnie | Opis |
|---|---|---|
| `interval` | `300` | Interwał przesyłania w sekundach |
| `keypair` | `~/.config/strontium/oracle-keypair.json` | Ścieżka oracle keypair |
| `vote_keypair` | auto-detect | Ścieżka vote keypair |
| `rpc` | localhost + mainnet | Dodaj endpoint RPC |
| `committee` | *(puste = solo)* | Dodaj oracle pubkey do komitetu rotacji |
| `committee_clear` | — | Wyczyść listę komitetu |
| `dry_run` | `false` | Tryb testowy (true/false) |
| `memo` | `true` | Dołącz Memo Program do TX (false = mniej compute units) |

---

## Rotacja — Podział kosztów

Daemon używa deterministycznej rotacji round-robin — **żadna komunikacja między serwerami nie jest potrzebna**:

```
window_id = aktualny_czas / interval_s
primary   = window_id % rozmiar_komitetu
```

**Stopniowy fallback:**
- `t + 0s` → primary przesyła
- `t + 30s` → backup-1 przesyła jeśli primary milczał
- `t + 60s` → backup-2 przesyła jeśli nadal brak

```bash
x1sr config set committee <ORACLE_PUBKEY_PRIME>
x1sr config set committee <ORACLE_PUBKEY_SENTINEL>
x1sr config show
```

---

## Koszt i dokładność

Każda transakcja kosztuje **0.002 XNT** (zweryfikowane on-chain). Użyj **[interaktywnego kalkulatora kosztów](https://piowin-clo.github.io/strontium)**.

Tabela (koszt per operator):

| Operatorów | Interwał | TX/dzień/op | XNT/miesiąc/op | Dokładność on-chain |
|---|---|---|---|---|
| 1 (solo) | 300s | 288 | ~17.3 XNT | ±3–10 ms |
| 2 (komitet) | 300s | 144 | ~8.6 XNT | ±2–6 ms |
| 5 (komitet) | 300s | 58 | ~3.5 XNT | ±2–6 ms |
| 10 (komitet) | 300s | 29 | ~1.7 XNT | ±2–5 ms |
| 50 (komitet) | 300s | 6 | ~0.35 XNT | ±1–4 ms |
| dowolnie + GPS/PPS | dowolnie | — | — | ±50 nanosekund |

> Gdy cena XNT rośnie, właściwą odpowiedzią jest więcej operatorów dzielących koszty — nie degradacja usługi przez zwiększanie interwału.

---

## Źródła NTP (21 łącznie)

| Tier | Źródło | Typ | Region |
|---|---|---|---|
| **T-0 GPS** | `/dev/pps0` | Sprzęt GPS/PPS | Lokalny serwer |
| **T-1 NTS** | `ptbtime1.ptb.de` | Atomowy + NTS | Niemcy |
| **T-1 NTS** | `time.cloudflare.com` | Komercyjny + NTS | Globalny |
| **T-1 NTS** | `ntp.time.nl` | Atomowy + NTS | Holandia |
| **T-2 S1** | `nts.netnod.se` | Atomowy Stratum-1 | Szwecja |
| **T-2 S1** | `ptbtime2.ptb.de` | Rządowy atomowy | Niemcy |
| **T-2 S1** | `ptbtime3.ptb.de` | Rządowy atomowy | Niemcy |
| **T-2 S1** | `tempus1.gum.gov.pl` | Rządowy atomowy | Polska |
| **T-2 S1** | `tempus2.gum.gov.pl` | Rządowy atomowy | Polska |
| **T-2 S1** | `tempus3.gum.gov.pl` | Rządowy atomowy | Polska |
| **T-2 S1** | `nist1-atl.ustiming.org` | Rządowy atomowy | USA |
| **T-2 S1** | `time.nist.gov` | Rządowy atomowy | USA |
| **T-2 S1** | `ntp.jst.mfeed.ad.jp` | Rządowy atomowy | Japonia |
| **T-2 S1** | `syrte.obspm.fr` | Rządowy atomowy | Francja |
| **T-2 S1** | `ntp-p1.obspm.fr` | Rządowy atomowy | Francja |
| **T-2 S1** | `ntp.metas.ch` | Rządowy atomowy | Szwajcaria |
| **T-2 S1** | `time.google.com` | Komercyjny | Globalny |
| **T-2 S1** | `ntp.nic.cz` | Rządowy Stratum-1 | Czechy |
| **T-2 S1** | `ntp1.fau.de` | Akademicki | Niemcy |
| **T-3 Pool** | `0.pool.ntp.org` | Społecznościowy | Globalny |
| **T-3 Pool** | `1.pool.ntp.org` | Społecznościowy | Globalny |
| **T-3 Pool** | `europe.pool.ntp.org` | Społecznościowy | Europa |

Wszystkie źródła odpytywane równolegle. Lista odświeżana co godzinę. Deduplikacja po IP (ochrona przed anycast). Daemon wybiera 5 najlepszych źródeł na cykl, wymagając co najmniej 3 Stratum-1 lub lepszych.

**GPS/PPS (opcjonalne):** Daemon automatycznie wykrywa `/dev/pps0` przy starcie. Jeśli nieobecny — automatyczny fallback do NTP, zero konfiguracji, zero błędów. Polecany sprzęt: u-blox NEO-M8N (~$30 USB).

---

## Adresy On-Chain

| | Adres |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [Zobacz na X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Rozwiązywanie problemów

| Powód milczenia | Co zrobić |
|---|---|
| `no_valid_sources` | Sprawdź port 123/UDP: `nc -zu pool.ntp.org 123` |
| `spread_too_high` | Źródła NTP różnią się o >50ms — poczekaj |
| `low_confidence` | Za mało źródeł — sprawdź `x1sr sources` |
| `not_elected` | Rotacja: okno innego walidatora — normalne |
| `registration_expired` | Uruchom `x1sr register` ponownie |
| `insufficient_balance` | Doładuj oracle keypair |
| `dry_run` | Tryb testowy — zrestartuj bez `--dry-run` |
| `timestamp_outlier` | Twój czas NTP odbiega o >10s od zegara sieci — sprawdź chrony |

**Błędy rejestracji:**

| Błąd | Rozwiązanie |
|---|---|
| `AccountNotFound` | Doładuj oracle keypair (Krok 3) |
| `AccountNotSigner` | Sprawdź ścieżkę `--vote-keypair` |
| `Insufficient self-stake` | Zwiększ self-stake do ≥100 XNT przez XDEX Valistake |
| `Skip rate too high` | Poczekaj aż skip rate walidatora spadnie poniżej 10% |

---

## Bezpieczeństwo

**Upgrade authority:** `7k4tvn5Aim8yWEdSAfZqptTvTf7r1WXUNSNa8evmmNGq` (Ledger — cold storage)

Upgrade programu wymaga fizycznego potwierdzenia na Ledgerze. Klucz fee-payer oracle (`EgFaM42n...`) nie ma możliwości upgrade kontraktu.

| Atak | Zabezpieczenie |
|---|---|
| Kłamstwo pojedynczego walidatora | Check outlier on-chain: odrzucenie jeśli >10s od `Clock` |
| Skoordynowana manipulacja timestampem | Mediana ważona stake — wymaga skompromitowania większości |
| MITM na NTP | Cross-check między kontynentami + walidacja między tierami |
| Spam zgłoszeń | Wymagana ValidatorRegistration (dowód vote + stake) |
| Kradzież klucza oracle | Tylko oracle keypair narażony — identity/vote/upgrade authority bezpieczne |
| Atak przez upgrade | Upgrade authority na zimnym Ledgerze |

---

## Checklist przed wdrożeniem produkcyjnym

- [ ] Oracle keypair doładowany (`x1sr balance` — co najmniej 30 dni runway)
- [ ] Rejestracja potwierdzona (`x1sr status` pokazuje `running`)
- [ ] Dry-run zakończony sukcesem przez co najmniej 3 cykle
- [ ] Źródła NTP odpowiadają (`x1sr sources` pokazuje ≥3 aktywne)
- [ ] Port 123/UDP otwarty wychodzący
- [ ] Komitet skonfigurowany na wszystkich węzłach (jeśli używasz rotacji)
- [ ] Failover przetestowany: zatrzymaj primary, zweryfikuj że backup przesyła w ciągu 60s
- [ ] Usługa systemd zainstalowana (`sudo x1sr install`)

---

## Mapa drogowa

- [x] Równoległe odpytywanie NTP z 4-tierową klasyfikacją (21 serwerów, 4 kontynenty)
- [x] Bufor pierścieniowy on-chain (288 slotów, `zero_copy`)
- [x] ValidatorRegistration — dowód vote account + weryfikacja stake + TTL 90d
- [x] `sources_bitmap` per zgłoszenie — pełna audytowalność
- [x] Scoring pewności
- [x] Pełne CLI (`start`, `stop`, `status`, `sources`, `config`, `install`, ...)
- [x] Automatyczny instalator systemd
- [x] Memo Program opcjonalne (config set memo false)
- [x] Circuit breaker RPC z exponential backoff
- [x] Deterministyczna rotacja round-robin — sprawiedliwy podział kosztów
- [x] Deduplikacja po IP — ochrona przed anycast pool
- [x] Walidacja konsensusu między tierami
- [x] Outlier slashing on-chain — odrzucaj zgłoszenia >10s od `Clock`
- [x] Upgrade authority na zimnym Ledgerze
- [x] Interaktywny kalkulator kosztów — [piowin-clo.github.io/strontium](https://piowin-clo.github.io/strontium)
- [ ] GPS/PPS przetestowane produkcyjnie (wymagany sprzęt)
- [ ] Pełny protokół NTS po stronie klienta (kryptograficzny handshake)
- [ ] Dashboard — wizualizacja konsensusu i historii
- [ ] Integracja z Alpenglow (τₖ phase-lock — brakująca warstwa czasu dla eigenvm)

---

## Zbudowane na X1

X1 Strontium to open-source infrastruktura dla ekosystemu X1. Zbudowane z Anchor 0.31.1 na Tachyon 2.2.20. CI: Build + Clippy + Audyt bezpieczeństwa przy każdym commicie.

**Na ramionach gigantów:** X1 Strontium powstał niezależnie, ale nie mógłby istnieć bez wizji Jacka Levina i pracy całego zespołu X1 — Photon Oracle, Entropy Engine i samego blockchaina X1. Jack i jego zespół zbudowali fundament. My zbudowaliśmy na nim.

**Koncepcja i architektura:** PioWin
**Kod:** Claude (Anthropic) przy wsparciu Theo (Cyberdyne)
