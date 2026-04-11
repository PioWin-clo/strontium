# ⚛️ X1 Strontium

**Atomowa dokładność czasu dla blockchaina X1.**

[![CI](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml/badge.svg)](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml)
[![Built on X1](https://img.shields.io/badge/Built%20on-X1-black)](https://x1.xyz)

> Zielona odznaka CI powyżej oznacza że kod buduje się poprawnie, przechodzi linting i audyt bezpieczeństwa przy każdym commicie.

🇬🇧 [English](README.md) | 🇵🇱 Polski

X1 Strontium to zdecentralizowany oracle czasu NTP dla [blockchaina X1](https://x1.xyz). Dostarcza kryptograficznie potwierdzonych znaczników czasu UTC zapisanych on-chain — zebranych z zegarów atomowych, komercyjnych dostawców NTP i puli społecznościowych — weryfikowanych przez sieć walidatorów.

---

## Problem

Na Solana/X1 `Clock::unix_timestamp` jest raportowany przez lidera bloku — może być manipulowany o ±1–2 sekundy bez wykrycia. Dla większości transakcji to nieistotne. Ale dla:

- **Kontraktów vestingowych** — dokładny moment wypłaty
- **Aukcji subsekudowych** — kto wygrał?
- **Dowodów czasu między łańcuchami** — weryfikowalne między sieciami
- **Kontraktów SLA** — znaczniki czasu uznawane przez sąd

...czas raportowany przez lidera to poważna luka bezpieczeństwa. X1 Strontium rozwiązuje ten problem.

---

## Jak to działa

Każdy zarejestrowany walidator uruchamia lekkiego daemona Strontium obok Tachyona. Co **5 minut** (domyślnie, konfigurowalnie):

1. Odpytuje równolegle do 17 serwerów NTP z całego świata — mix: zegary atomowe, komercyjne, pule społecznościowe z 4 kontynentów
2. Wybiera 5 najlepszych źródeł wg tieru (GPS/PPS → NTS → Stratum-1 → Pool) i czasu odpowiedzi (RTT)
3. Oblicza medianę skorygowaną o RTT i waliduje rozpiętość (próg: ±50ms)
4. Oblicza **wynik pewności**: `liczba_źródeł × 0.4 + jakość_rozpiętości × 0.4 + waga_tieru × 0.2`
5. Jeśli pewność ≥ 0.60 → wysyła znacznik czasu on-chain (dwie instrukcje: `submit_time` + Memo Program)
6. Jeśli źródła się nie zgadzają → **milczy** (cisza = wbudowana tolerancja na kłamstwa)

Każde zgłoszenie zawiera `sources_bitmap` — każda runda jest w pełni audytowalna on-chain.

Program on-chain agreguje zgłoszenia do **bufora 288 slotów** przez medianę ważoną stake. Zmanipulowanie wyniku wymaga przejęcia kontroli nad większością submitterów jednocześnie.

> **Dlaczego mix źródeł a nie tylko rządowe zegary atomowe?**
> Sieć jest zdecentralizowana — nie chcemy zależeć od jednego kraju ani jednej instytucji.
> Każde źródło to jeden głos. Mediana eliminuje kłamców. Im więcej niezależnych źródeł, tym większa odporność.

---

## Architektura

```
Serwer walidatora                        Blockchain X1
┌──────────────────────────┐           ┌─────────────────────────────────┐
│   Walidator Tachyon      │           │                                 │
│                          │           │  OracleState PDA                │
│   Daemon Strontium       │──TX+Memo─▶│  ┌───────────────────────────┐  │
│   ┌────────────────────┐ │           │  │  trusted_time_ms          │  │
│   │  Autodiscovery NTP │ │           │  │  spread_ms                │  │
│   │  ┌──────────────┐  │ │           │  │  confidence               │  │
│   │  │ GPS/PPS  t-0 │  │ │           │  │  sources_bitmap           │  │
│   │  │ NTS      t-1 │  │ │           │  │  ring_buffer[288]         │  │
│   │  │ Stratum1 t-2 │  │ │           │  └───────────────────────────┘  │
│   │  │ Pool     t-3 │  │ │           │                                 │
│   │  └──────────────┘  │ │           │  ValidatorRegistration PDA      │
│   │  Wątki równoległe  │ │           │  (TTL: 90 dni, weryfikacja stake)│
│   └────────────────────┘ │           │                                 │
└──────────────────────────┘           └─────────────────────────────────┘
```

Każda transakcja zawiera dwie instrukcje:
- `submit_time` → zapisuje dane do bufora pierścieniowego on-chain
- `Memo Program` → czytelny log: `strontium:v1:w={okno}:t={czas}:c={pewność}:s={źródła}`

Każde zgłoszenie jest widoczne w eksploratorze i możliwe do audytu.

---

## Wymagania

| Wymaganie | Szczegóły |
|---|---|
| **System** | Ubuntu 22.04 LTS lub nowszy (GLIBC 2.35+) |
| **Solana CLI** | Zainstalowane i w PATH (musi działać `solana-keygen`) |
| **Saldo XNT** | ≥1 XNT na oracle keypair |
| **Self-stake** | ≥100 XNT zweryfikowane na walidatorze |
| **Skip rate** | <10% (sprawdzane przy rejestracji) |
| **Sieć** | Port 123/UDP otwarty wychodzący (NTP) |
| **Status** | Walidator aktywny na mainnecie |

> **Sprawdź port 123 UDP:**
> ```bash
> nc -zu pool.ntp.org 123 && echo "OK — port otwarty" || echo "ZABLOKOWANY — odblokuj: sudo ufw allow out 123/udp"
> ```

> **Inne dystrybucje Linux:** Skompiluj ze źródeł:
> ```bash
> git clone https://github.com/PioWin-clo/strontium
> cd strontium/daemon && cargo build --release
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

> ⚠️ **Tylko NOWY dedykowany keypair.** NIE używaj `identity.json` ani `vote.json`.
> Jeśli zostanie skompromitowany — stracisz tylko saldo oracle, walidator pozostaje bezpieczny.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

### Krok 3 — Zasil oracle keypair

Wyślij minimum **1 XNT** z dowolnego portfela — nie ma znaczenia skąd:

```bash
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json
# Następnie wyślij XNT na ten adres przez XDEX, Backpack, CLI lub Ledger
```

Zobacz tabelę kosztów poniżej żeby dobrać odpowiednią kwotę do wybranego interwału.

### Krok 4 — Rejestracja

> ⚠️ `vote.json` to keypair vote account Twojego walidatora — leży na serwerze w `~/.config/solana/vote.json`. To NIE jest klucz Ledgera.

```bash
x1sr register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Rejestracja weryfikuje: walidator aktywny, skip rate <10%, self-stake ≥100 XNT.

> Rejestracja wygasa po **90 dniach** — zarejestruj się ponownie przed wygaśnięciem.

### Krok 5 — Uruchom daemona

**Tryb testowy** (tylko konsensus NTP, zero transakcji, zero kosztów):
```bash
x1sr start --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Tryb live** (submittuje co 5 minut):
```bash
nohup x1sr start \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

```bash
x1sr status
tail -f ~/strontium.log
# Powinieneś widzieć: ✅ submit OK — tx: ...
```

### Krok 6 — Zainstaluj jako serwis systemowy

```bash
x1sr install
```

Automatycznie wykrywa nazwę użytkownika i ścieżkę binarki, sprawdza saldo, generuje i włącza `/etc/systemd/system/strontium.service`.

---

## Dokumentacja CLI

```
x1sr start            Uruchom daemona (tryb live)
x1sr start --dry-run  Uruchom w trybie testowym
x1sr stop             Zatrzymaj daemona
x1sr status           Status, konsensus NTP, saldo, rotacja
x1sr sources          Tabela źródeł NTP (RTT, offset, tier, NTS)
x1sr history [N]      Ostatnie N zgłoszeń on-chain (domyślnie: 10)
x1sr register         Zarejestruj oracle walidatora
x1sr deregister       Wyrejestruj (wkrótce)
x1sr balance          Saldo oracle keypair i prognoza
x1sr archive          Eksportuj historię on-chain do JSONL
x1sr config show      Pokaż aktualną konfigurację
x1sr config set K V   Ustaw parametr konfiguracji
x1sr install          Zainstaluj jako serwis systemd
x1sr uninstall        Usuń serwis systemd
```

**Parametry konfiguracji** (`x1sr config set <klucz> <wartość>`):

| Klucz | Domyślnie | Opis |
|---|---|---|
| `interval` | `300` | Interwał zgłoszeń w sekundach |
| `keypair` | `~/.config/strontium/oracle-keypair.json` | Ścieżka do oracle keypair |
| `vote_keypair` | auto-detect | Ścieżka do vote keypair |
| `rpc` | localhost + mainnet | Dodaj endpoint RPC |
| `committee` | *(puste = solo)* | Dodaj pubkey oracle do listy rotacji |
| `committee_clear` | — | Wyczyść listę committee |
| `dry_run` | `false` | Tryb testowy (true/false) |

---

## Rotacja — podział kosztów

Kilku walidatorów może koordynować zgłoszenia żeby dzielić koszty i poprawić pokrycie czasowe. Daemon używa deterministycznej rotacji round-robin — **żadna komunikacja między serwerami nie jest potrzebna**:

```
window_id = aktualny_czas / interwał_s
primary   = window_id % liczba_w_committee
```

Każdy daemon niezależnie oblicza czyja kolej. Szybszy serwer ani lepsze łącze nie dają żadnej przewagi — wynik jest identyczny dla wszystkich.

**Stagowane zapasowe** (zapobiega przerwom gdy primary jest offline):
- `t + 0s` → primary submittuje
- `t + 20s` → backup-1 submittuje jeśli primary milczał
- `t + 40s` → backup-2 submittuje jeśli nadal cisza

**Jak skonfigurować rotację:**

```bash
# Dodaj oba oracle pubkeys do committee (to samo na obu serwerach)
x1sr config set committee <ORACLE_PUBKEY_PRIME>
x1sr config set committee <ORACLE_PUBKEY_SENTINEL>

# Sprawdź
x1sr config show
```

Lista jest automatycznie sortowana — kolejność dodawania nie ma znaczenia. Zrestartuj daemona po zmianach.

---

## Koszty i dokładność

Każda transakcja kosztuje **0.002 XNT**. Więcej operatorów = niższy koszt per operator = możliwy krótszy interwał = lepsza dokładność czasu on-chain:

| Operatorów | Interwał | TX/dzień/operator | XNT/mies./operator | Dokładność on-chain |
|---|---|---|---|---|
| 1 | 300s | 288 | ~17.3 XNT | ±3–10 ms |
| 2 | 300s | 144 | ~8.6 XNT | ±2–6 ms |
| 5 | 300s | 58 | ~3.5 XNT | ±2–6 ms |
| 10 | 120s | 72 | ~4.3 XNT | ±2–5 ms |
| 50 | 60s | 29 | ~1.7 XNT | ±1–4 ms |
| 100+ | 30s | 25 | ~1.5 XNT | ±1–4 ms |
| dowolnie + GPS/PPS | dowolnie | — | — | ±50 nanosekund |

> Im więcej operatorów dołącza, tym krótszy interwał każdy może sobie pozwolić — poprawiając dokładność dla całej sieci przy tym samym indywidualnym koszcie.

Zmień interwał:
```bash
x1sr config set interval 600    # co 10 minut
x1sr config set interval 3600   # co godzinę
```

---

## Źródła NTP (17 łącznie)

| Tier | Źródło | Typ | Lokalizacja |
|---|---|---|---|
| **T-0 GPS** | `/dev/pps0` | GPS/PPS sprzętowy | Lokalny serwer |
| **T-1 NTS** | `ptbtime1.ptb.de` | Atomowy + NTS | Niemcy |
| **T-1 NTS** | `time.cloudflare.com` | Komercyjny + NTS | Globalny |
| **T-1 NTS** | `nts.netnod.se` | Atomowy + NTS | Szwecja |
| **T-2 S1** | `ptbtime2/3.ptb.de` | Rządowy atomowy | Niemcy |
| **T-2 S1** | `tempus1/2/3.gum.gov.pl` | Rządowy atomowy | Polska |
| **T-2 S1** | `nist1-atl`, `time.nist.gov` | Rządowy atomowy | USA |
| **T-2 S1** | `syrte.obspm.fr`, `ntp.metas.ch` | Rządowy atomowy | Francja, Szwajcaria |
| **T-2 S1** | `ntp.jst.mfeed.ad.jp` | Rządowy atomowy | Japonia |
| **T-2 S1** | `time.google.com` | Komercyjny | Globalny |
| **T-3 Pool** | `{0,1}.pool.ntp.org` | Społecznościowy | Globalny |
| **T-3 Pool** | `europe.pool.ntp.org` | Społecznościowy | Europa |

Wszystkie odpytywane równolegle. Lista odświeżana co godzinę. GPS/PPS wykrywany automatycznie przez `/dev/pps0`.

---

## Adresy on-chain

| | Adres |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [Zobacz na X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Jak odczytać czas on-chain

Każde zgłoszenie zawiera Memo czytelny w eksploratorze:
```
strontium:v1:w=1234:t=1712780400000:c=87:s=5
```
gdzie: `w` = numer okna, `t` = czas Unix w ms, `c` = pewność (0–100), `s` = liczba źródeł.

Wszystkie zgłoszenia: [X1 Explorer — Oracle PDA](https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn)

Dla integracji przez Anchor odczytaj konto `OracleState` pod adresem Oracle PDA i użyj `latest_trusted_time_ms`. Sprawdź `staleness_slots` względem swojego maksymalnego dopuszczalnego opóźnienia przed użyciem wartości.

---

## Rozwiązywanie problemów

**Daemon milczy przez wiele cykli:**
```bash
x1sr status    # sprawdź pole silent_reason
x1sr sources   # sprawdź które serwery NTP odpowiadają
```

| Powód milczenia | Co zrobić |
|---|---|
| `no_valid_sources` | Sprawdź port 123/UDP: `nc -zu pool.ntp.org 123` |
| `spread_too_high` | Serwery NTP różnią się o >50ms — poczekaj |
| `low_confidence` | Za mało jakościowych źródeł — sprawdź `x1sr sources` |
| `not_elected` | Rotacja: okno innego walidatora — normalne zachowanie |
| `registration_expired` | Uruchom `x1sr register` ponownie (TTL 90 dni) |
| `insufficient_balance` | Zasil oracle keypair |
| `dry_run` | Tryb testowy aktywny — uruchom bez `--dry-run` |

**Błędy przy rejestracji:**

| Błąd | Rozwiązanie |
|---|---|
| `AccountNotFound` | Zasil oracle keypair (Krok 3) |
| `AccountNotSigner` | Sprawdź ścieżkę `--vote-keypair` |
| `Insufficient self-stake` | Zwiększ self-stake do ≥100 XNT przez XDEX Valistake |
| `Skip rate too high` | Poczekaj aż skip rate spadnie poniżej 10% |

**Binary nie uruchamia się (`GLIBC not found`):**
```bash
git clone https://github.com/PioWin-clo/strontium
cd strontium/daemon && cargo build --release
./target/release/strontium help
```

---

## Bezpieczeństwo

**Upgrade authority:** `EgFaM42nFeZYwDXzMZWNTmp5ojyL7UGP8xgdX1SBXYsb`

| Atak | Ochrona |
|---|---|
| Kłamstwo jednego walidatora | Mediana ważona stake — wymaga większości submitterów |
| Atak MITM na NTP | Multi-kontynentalny cross-check (próg 50ms) |
| Spam zgłoszeń | ValidatorRegistration wymagany (dowód vote + stake) |
| Kompromitacja oracle keypair | Tylko oracle keypair zagrożony — identity/vote nienaruszone |
| Spoofing GPS | Cross-check z konsensusem NTP (próg ±5s) |

**Odpowiedzialne ujawnianie:** [GitHub Issues](https://github.com/PioWin-clo/strontium/issues) lub Telegram grupy X1 Validator Army.

---

## Roadmapa

- [x] Równoległe zapytania NTP z 4-tierową klasyfikacją źródeł
- [x] Bufor pierścieniowy on-chain (288 slotów, `zero_copy`)
- [x] ValidatorRegistration — dowód vote account + weryfikacja stake + TTL 90d
- [x] `sources_bitmap` per zgłoszenie — pełna audytowalność
- [x] Wynik confidence (pewności)
- [x] Pełne CLI (`start`, `stop`, `status`, `sources`, `config`, `install`, ...)
- [x] Automatyczny instalator systemd
- [x] Memo Program w każdej transakcji — pełna transparentność
- [x] Circuit breaker RPC z exponential backoff
- [x] Deterministyczna rotacja round-robin (`slot % n`) — podział kosztów
- [x] ed25519-dalek v2, czysty Clippy, audit bezpieczeństwa
- [ ] Dashboard — wizualizacja konsensusu, historia, health walidatorów
- [ ] Egzekwowanie progu stake on-chain
- [ ] Pełny protokół NTS po stronie klienta
- [ ] GPS/PPS — produkcyjnie przetestowane
- [ ] Integracja Alpenglow (τₖ phase-lock — brakująca warstwa czasu dla eigenvm)

---

## Zbudowane na X1

X1 Strontium to open-source infrastruktura dla ekosystemu X1.
Zbudowane z Anchor 0.31.1 na Tachyon 2.2.20. CI: Build + Clippy + Security audit na każdym commicie.

**Na barkach otwartego kodu:** X1 Strontium powstało jako niezależny pomysł, ale nie istniałoby bez wizji Jacka Levina i pracy całego zespołu X1 — Photon Oracle, Entropy Engine i samego blockchaina X1. Jack i jego zespół zbudowali fundamenty. My zbudowaliśmy na nich.

**Pomysł i architektura:** PioWin
**Kod:** Claude (Anthropic) przy wsparciu Theo (Cyberdyne)
