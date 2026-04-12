# ⚛️ X1 Strontium

**Atomowa precyzja czasu dla blockchaina X1.**

[![CI](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml/badge.svg)](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml)
[![Built on X1](https://img.shields.io/badge/Built%20on-X1-black)](https://x1.xyz)

> Zielona odznaka CI oznacza że kod się kompiluje, przechodzi linting i audyt bezpieczeństwa przy każdym commicie.

🇬🇧 [English](README.md) | 🇵🇱 Polski

X1 Strontium to zdecentralizowany time oracle NTP dla [blockchaina X1](https://x1.xyz). Dostarcza kryptograficznie poświadczone znaczniki czasu UTC on-chain, pochodzące z zegarów atomowych, komercyjnych dostawców NTP i pul społecznościowych — weryfikowane przez samą sieć walidatorów.

---

## Problem

Na Solana/X1, `Clock::unix_timestamp` jest podawany przez lidera bloku — może być manipulowany o ±1–2 sekundy bez wykrycia przez sieć. Dla większości transakcji jest to nieistotne. Ale dla:

- **Kontraktów vestingowych** — dokładny termin wypłaty
- **Aukcji sub-sekundowych** — kto wygrał?
- **Dowodów czasu cross-chain** — weryfikowalnych między sieciami
- **Kontraktów SLA** — znaczniki czasu ważne prawnie

...czas raportowany przez lidera to poważna podatność. X1 Strontium to naprawia.

---

## Jak to działa

Każdy zarejestrowany walidator uruchamia lekki daemon Strontium obok Tachyona. Co **5 minut** (konfigurowalnie):

1. Odpytuje równolegle wszystkie 21 serwerów NTP — zegary atomowe, komercyjni dostawcy, pule społecznościowe z 4 kontynentów
2. Wybiera 5 najlepszych źródeł według tieru (GPS/PPS → NTS → Stratum-1 → Pool) i RTT, deduplikując po rozwiązanym IP
3. Oblicza medianę skorygowaną o RTT i waliduje spread (próg: ±50ms)
4. Waliduje konsensus między tierami — co najmniej 2 niezależne tiery muszą zgadzać się w ±60ms
5. Oblicza **wskaźnik pewności**: `źródła × 0.4 + jakość_spreadu × 0.4 + waga_tieru × 0.2`
6. Sprawdza przesłany timestamp względem zegara on-chain — odrzuca jeśli odchylenie > 10 sekund
7. Jeśli pewność ≥ 0.60 → wysyła timestamp on-chain przez `submit_time` + opcjonalnie Memo Program
8. Jeśli źródła się nie zgadzają → **milczy** (cisza jako sygnał = ochrona przed błędami bizantyjskimi)

Każda submisja zawiera `sources_bitmap` — każda runda jest w pełni audytowalna on-chain. Program on-chain agreguje submisje w **ring buffer 288 slotów** przez medianę ważoną stake'iem. Manipulacja wynikiem wymaga skompromitowania większości submiterów jednocześnie.

> **Dlaczego mix źródeł, a nie tylko zegary rządowe?**
> Sieć jest zdecentralizowana — nie chcemy zależeć od jednego kraju ani instytucji.
> Każde źródło to jeden głos. Mediana eliminuje kłamców. Więcej niezależnych źródeł = silniejsza odporność.

---

## Architektura

```
Serwer walidatora                      Blockchain X1
┌──────────────────────────┐       ┌─────────────────────────────────┐
│  Tachyon Validator       │       │                                 │
│                          │       │  OracleState PDA                │
│  Strontium Daemon ──TX──▶│       │ ┌───────────────────────────┐   │
│  ┌────────────────────┐  │       │ │  trusted_time_ms          │   │
│  │  Autodiscovery NTP │  │       │ │  spread_ms                │   │
│  │  ┌──────────────┐  │  │       │ │  confidence               │   │
│  │  │  GPS/PPS t-0 │  │  │       │ │  sources_bitmap           │   │
│  │  │  NTS     t-1 │  │  │       │ │  ring_buffer[288]         │   │
│  │  │  Stratum1 t-2│  │  │       │ └───────────────────────────┘   │
│  │  │  Pool    t-3 │  │  │       │                                 │
│  │  └──────────────┘  │  │       │  ValidatorRegistration PDA      │
│  │  Równoległe zapyt. │  │       │  (TTL: 90 dni, weryfikacja stake)│
│  └────────────────────┘  │       │                                 │
└──────────────────────────┘       └─────────────────────────────────┘
```

Każda transakcja zawiera dwie instrukcje (memo opcjonalne):
- `submit_time` → zapisuje do ring buffera on-chain, sprawdzenie outlierów względem `Clock`
- `Memo Program` → czytelny log: `strontium:v1:w={okno}:t={czas}:c={pewność}:s={źródła}`

---

## Wymagania

| Wymaganie | Szczegóły |
|---|---|
| **System** | Ubuntu 18.04 lub nowszy, dowolny Linux x86_64 |
| **Solana CLI** | Zainstalowane i w PATH (`solana-keygen` musi działać) |
| **Saldo XNT** | ≥1 XNT na keypairze oracle |
| **Self-stake** | ≥100 XNT zweryfikowane na walidatorze |
| **Skip rate** | <10% (sprawdzane przy rejestracji) |
| **Sieć** | Port 123/UDP otwarty wychodzący (NTP) |
| **Status walidatora** | Aktywny na mainnecie |

> **Sprawdź port 123 UDP:**
> ```bash
> nc -zu pool.ntp.org 123 && echo "OK — port otwarty" || echo "ZABLOKOWANY — otwórz: sudo ufw allow out 123/udp"
> ```
> **Kompilacja ze źródeł (dowolna dystrybucja Linux):**
> ```bash
> git clone https://github.com/PioWin-clo/strontium
> cd strontium/daemon && cargo build --release
> ```

---

## Szybki start

### Krok 1 — Pobierz binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium-linux-x86_64 -O strontium
chmod +x strontium
sudo mv strontium /usr/local/bin/strontium
sudo ln -sf /usr/local/bin/strontium /usr/local/bin/x1sr
x1sr help
```

> **Statyczne binary** — działa na Ubuntu 18.04, 20.04, 22.04, 24.04, Debian, CentOS i każdym Linux x86_64 bez wymagań dotyczących wersji GLIBC.

### Krok 2 — Wygeneruj keypair oracle

> ⚠️ **Tylko NOWY, dedykowany keypair.** NIE używaj `identity.json` ani `vote.json`.
> W razie kompromitacji zagrożone jest tylko saldo keypair oracle — walidator pozostaje bezpieczny.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

### Krok 3 — Doładuj keypair oracle

```bash
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json
# Wyślij XNT na ten adres przez XDEX, Backpack, CLI lub Ledger
```

Użyj **[interaktywnego kalkulatora kosztów](https://piowin-clo.github.io/strontium)** żeby dobrać odpowiednią kwotę dla swojego interwału.

### Krok 4 — Zarejestruj się

> ⚠️ `vote.json` to keypair vote Twojego walidatora — znajduje się na serwerze w `~/.config/solana/vote.json`. NIE jest to Twój klucz Ledger withdraw.

```bash
x1sr register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Rejestracja weryfikuje: aktywność walidatora, skip rate <10%, self-stake ≥100 XNT.

> Rejestracja wygasa po **90 dniach** — odnów przez `x1sr register` przed wygaśnięciem.

### Krok 5 — Uruchom daemon

**Dry-run** (tylko konsensus NTP, brak transakcji on-chain, zero kosztów):

```bash
x1sr start --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Tryb live** (submisja co 5 minut):

```bash
nohup x1sr start \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

```bash
x1sr status
tail -f ~/strontium.log
# Powinieneś zobaczyć: ✅ submit OK — tx: ...
```

### Krok 6 — Zainstaluj jako usługę systemd

```bash
sudo x1sr install
```

Automatycznie wykrywa nazwę użytkownika i ścieżkę binary, sprawdza saldo, generuje i włącza `/etc/systemd/system/strontium.service`. Usługa czeka 2 minuty po bootowaniu przed startem (daje Tachyonowi czas na dołączenie do sieci).

---

## Dokumentacja CLI

```
x1sr start              Uruchom daemon (tryb live)
x1sr start --dry-run    Uruchom w trybie testowym (bez transakcji)
x1sr stop               Zatrzymaj daemon
x1sr status             Status, konsensus NTP, saldo, rotacja
x1sr sources            Tabela źródeł NTP (RTT, offset, tier, NTS)
x1sr history [N]        Ostatnie N submisji on-chain (domyślnie: 10)
x1sr register           Zarejestruj oracle walidatora
x1sr deregister         Wyrejestruj (wkrótce)
x1sr balance            Saldo keypair oracle i runway
x1sr archive            Eksport historii on-chain do JSONL
x1sr config show        Pokaż aktualną konfigurację
x1sr config set K V     Ustaw wartość konfiguracji
x1sr install            Zainstaluj jako usługę systemd (uruchom z sudo)
x1sr uninstall          Usuń usługę systemd
```

**Klucze konfiguracji** (`x1sr config set <klucz> <wartość>`):

| Klucz | Domyślnie | Opis |
|---|---|---|
| `interval` | `300` | Interwał submisji w sekundach |
| `keypair` | `~/.config/strontium/oracle-keypair.json` | Ścieżka do keypair oracle |
| `vote_keypair` | auto-detect | Ścieżka do keypair vote |
| `rpc` | localhost + mainnet | Dodaj endpoint RPC (dodawany na początku listy) |
| `rotation` | `true` | Auto-rotacja włączona (false = zawsze submituj) |
| `dry_run` | `false` | Tryb testowy (true/false) |
| `memo` | `true` | Dołącz Memo Program do TX (false = niższe compute units) |

---

## Jak koszty skalują się automatycznie

X1 Strontium używa **automatycznej rotacji opartej na slot hash** — bez żadnej ręcznej konfiguracji. Po rejestracji każdy daemon niezależnie odkrywa aktywnych oracle on-chain i rozdziela submisje równomiernie.

Im więcej walidatorów dołączy, tym niższy koszt dla każdego — **automatycznie, bez żadnej koordynacji**.

```
window_id = czas_unix / interwał
winner    = SHA256(slot_hash || window_id) % liczba_aktywnych_oracle
```

Każdy daemon niezależnie oblicza czyja kolej. Slot hash jest nieprzewidywalny — nikt nie może z góry zaplanować manipulacji rotacją.

**Staged fallback** (zapobiega lukom gdy primary jest offline):

- `t + 0s` → primary submituje
- `t + 30s` → backup-1 submituje jeśli primary milczał
- `t + 60s` → backup-2 submituje jeśli nadal cisza

**Tryb solo** jest automatyczny gdy mniej niż 2 oracle są aktywne — daemon wykrywa to i submituje każde okno bez czekania.

---

## Koszty i dokładność

Każda transakcja kosztuje **0.002 XNT** (zweryfikowane on-chain). Użyj **[interaktywnego kalkulatora kosztów](https://piowin-clo.github.io/strontium)** żeby modelować swój setup.

Szybka referencja (auto-rotacja, koszt per operator):

| Operatorzy | Interwał | XNT/mies/op | Dokładność on-chain |
|---|---|---|---|
| 1 | 300s | ~17.3 XNT | ±3–10 ms |
| 2 | 300s | ~8.6 XNT | ±2–6 ms |
| 5 | 300s | ~3.5 XNT | ±2–6 ms |
| 10 | 300s | ~1.7 XNT | ±2–5 ms |
| 50 | 300s | ~0.35 XNT | ±1–4 ms |
| dowolna + GPS/PPS | dowolny | — | ±50 nanosekund |

> Gdy cena XNT rośnie, właściwą odpowiedzią jest więcej operatorów dzielących koszty — nie degradacja usługi przez zwiększenie interwału.

Zmiana interwału:

```bash
x1sr config set interval 60    # co minutę
x1sr config set interval 600   # co 10 minut
x1sr config set interval 3600  # co godzinę
```

---

## Źródła NTP (21 łącznie)

| Tier | Źródło | Typ | Region |
|---|---|---|---|
| **T-0 GPS** | `/dev/pps0` | GPS/PPS sprzętowy | Serwer lokalny |
| **T-1 NTS** | `ptbtime1.ptb.de` | Atomowy + auth NTS | Niemcy |
| **T-1 NTS** | `time.cloudflare.com` | Komercyjny + auth NTS | Globalny |
| **T-1 NTS** | `ntp.time.nl` | Atomowy + auth NTS | Holandia |
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
| **T-2 S1** | `ntp1.fau.de` | Uczelniane atomowy | Niemcy |
| **T-3 Pool** | `0.pool.ntp.org` | Społecznościowy | Globalny |
| **T-3 Pool** | `1.pool.ntp.org` | Społecznościowy | Globalny |
| **T-3 Pool** | `europe.pool.ntp.org` | Społecznościowy | Europa |

Wszystkie źródła odpytywane równolegle. Lista odświeżana co godzinę. Źródła deduplikowane po rozwiązanym IP (ochrona przed anycast pool). Daemon wybiera 5 najlepszych źródeł na cykl według priorytetu tieru, potem RTT, wymagając co najmniej 3 Stratum-1 lub lepszych.

**GPS/PPS (opcjonalne):** Daemon automatycznie wykrywa `/dev/pps0` przy starcie. Jeśli obecny — GPS/PPS jest używany jako tier-0 (dokładność ±50ns) z NTP jako cross-check. Jeśli nieobecny — automatycznie przełącza się na NTP — **bez konfiguracji, bez błędów**. Zalecany sprzęt: u-blox NEO-M8N (~50 USD USB).

---

## Adresy on-chain

| | Adres |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [Zobacz w X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Odczyt czasu on-chain

Każda submisja jest widoczna w explorerze. Każda transakcja zawiera Memo:

```
strontium:v1:w=1234:t=1712780400000:c=87:s=5
```

gdzie: `w` = id okna, `t` = czas Unix w ms, `c` = pewność (0–100), `s` = użyte źródła.

Wszystkie submisje: [X1 Explorer — Oracle PDA](https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn)

Do integracji on-chain przez Anchor: odczytaj konto `OracleState` pod adresem Oracle PDA i użyj `latest_trusted_time_ms`. Sprawdź `staleness_slots` względem swojego maksymalnego akceptowalnego opóźnienia przed zaufaniem wartości.

---

## Rozwiązywanie problemów

**Daemon milczy przez wiele cykli:**

```bash
x1sr status      # sprawdź pole silent_reason
x1sr sources     # sprawdź które serwery NTP odpowiadają
```

| Powód milczenia | Co zrobić |
|---|---|
| `no_valid_sources` | Sprawdź port 123/UDP: `nc -zu pool.ntp.org 123` |
| `spread_too_high` | Źródła NTP rozbieżne o >50ms — poczekaj lub sprawdź łączność |
| `low_confidence` | Za mało źródeł wysokiej jakości — sprawdź `x1sr sources` |
| `not_elected` | Rotacja: okno innego walidatora — normalny stan |
| `registration_expired` | Uruchom `x1sr register` ponownie (TTL 90 dni) |
| `insufficient_balance` | Doładuj keypair oracle — sprawdź `x1sr balance` |
| `dry_run` | Tryb testowy aktywny — uruchom ponownie bez `--dry-run` |
| `timestamp_outlier` | Czas NTP odbiega o >10s od zegara chain — sprawdź chrony |

**Błędy rejestracji:**

| Błąd | Rozwiązanie |
|---|---|
| `AccountNotFound` | Doładuj keypair oracle (Krok 3) |
| `AccountNotSigner` | Sprawdź ścieżkę `--vote-keypair` |
| `Insufficient self-stake` | Zwiększ self-stake do ≥100 XNT przez XDEX Valistake |
| `Skip rate too high` | Poczekaj aż skip rate walidatora spadnie poniżej 10% |

**Binary nie uruchamia się:**

```bash
git clone https://github.com/PioWin-clo/strontium
cd strontium/daemon && cargo build --release
target/release/strontium help
```

---

## Bezpieczeństwo

**Upgrade authority:** `7k4tvn5Aim8yWEdSAfZqptTvTf7r1WXUNSNa8evmmNGq` (Ledger — cold storage)

Upgrade programu wymaga fizycznego potwierdzenia na Ledgerze. Klucz fee-payer oracle (`EgFaM42n...`) nie ma możliwości upgrade'u.

| Atak | Mitygacja |
|---|---|
| Jeden kłamiący walidator | Sprawdzanie outlierów on-chain: odrzucenie jeśli timestamp odbiega o >10s od `Clock` |
| Skoordynowana manipulacja czasu | Mediana ważona stake — wymaga skompromitowania większości submiterów |
| MITM NTP | Cross-check wielokontynentowy (próg 50ms) + walidacja cross-tier |
| Granie pod rotację | Entropia slot-hash — nieprzewidywalna do ~150ms przed commitem bloku |
| Spam submisji | Wymagana ValidatorRegistration (dowód vote + sprawdzenie stake) |
| Kompromitacja klucza oracle | Tylko keypair oracle zagrożony — identity/vote/upgrade authority nienaruszone |
| Spoofing GPS | Cross-check z konsensusem NTP (próg ±5s) |
| Atak przez upgrade programu | Upgrade authority na cold Ledgerze — żaden gorący klucz nie może dokonać upgrade'u |

**Odpowiedzialne ujawnianie:** [GitHub Issues](https://github.com/PioWin-clo/strontium/issues) lub Telegram X1 Validator Army.

---

## Checklist przed uruchomieniem produkcyjnym

- [ ] Keypair oracle doładowany (`x1sr balance` — co najmniej 30 dni runway)
- [ ] Rejestracja potwierdzona (`x1sr status` pokazuje `running`)
- [ ] Dry-run ukończony pomyślnie przez co najmniej 3 cykle
- [ ] Źródła NTP odpowiadają (`x1sr sources` pokazuje ≥3 aktywne)
- [ ] Port 123/UDP otwarty wychodzący
- [ ] Fallback przetestowany: zatrzymaj daemon, sprawdź czy backup submituje w 60s
- [ ] Usługa systemd zainstalowana (`sudo x1sr install`)

---

## Roadmapa

- [x] Równoległe odpytywanie NTP z 4-tierową klasyfikacją źródeł (21 serwerów, 4 kontynenty)
- [x] Ring buffer on-chain (288 slotów, `zero_copy`)
- [x] ValidatorRegistration — dowód vote account + sprawdzenie stake + TTL 90d
- [x] `sources_bitmap` per submisja — pełna audytowalność
- [x] Scoring pewności
- [x] Pełne CLI (`start`, `stop`, `status`, `sources`, `config`, `install`, ...)
- [x] Automatyczny instalator systemd
- [x] Memo Program w każdej transakcji — pełna przejrzystość (opcjonalne przez config)
- [x] Circuit breaker RPC z exponential backoff
- [x] Walidacja cross-tier (co najmniej 2 niezależne tiery muszą się zgadzać)
- [x] Deduplikacja IP — ochrona przed anycast pool
- [x] Outlier slashing on-chain — odrzucenie submisji >10s od `Clock`
- [x] Upgrade authority na cold Ledger
- [x] Interaktywny kalkulator kosztów — [piowin-clo.github.io/strontium](https://piowin-clo.github.io/strontium)
- [x] **Auto-rotacja** — slot-hash based, zero konfiguracji, automatyczny podział kosztów
- [x] Statyczne binary — działa na wszystkich Linux x86_64 (musl, bez wymagań GLIBC)
- [ ] GPS/PPS przetestowane produkcyjnie (wymagany sprzęt: ~50 USD u-blox NEO-M8N)
- [ ] Pełny protokół NTS po stronie klienta (kryptograficzny handshake)
- [ ] Dashboard — wizualizacja konsensusu, historia, health walidatorów
- [ ] Odkrywanie oracle z chain (v2 — umożliwia pełną auto-rotację z listą live oracle)
- [ ] Integracja Alpenglow (τₖ phase-lock — brakująca warstwa czasu dla eigenvm)

---

## Zbudowane na X1

X1 Strontium to open-source infrastruktura dla ekosystemu X1. Zbudowane z Anchor 0.31.1 na Tachyon 2.2.20. CI: Build + Clippy + audyt bezpieczeństwa przy każdym commicie.

**Stojąc na ramionach gigantów:** X1 Strontium zostało zaprojektowane niezależnie, ale nie mogłoby istnieć bez wizji Jacka Levina i pracy całego zespołu X1 — Photon Oracle, Entropy Engine i samego blockchaina X1. Jack i jego zespół zbudowali fundament. My zbudowaliśmy na nim.

**Koncepcja i architektura:** PioWin  
**Kod:** Claude (Anthropic) ze wsparciem Theo (Cyberdyne)
