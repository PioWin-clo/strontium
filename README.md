# ⚛️ X1 Strontium

**Atomic-grade time for the X1 blockchain.**

[![CI](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml/badge.svg)](https://github.com/PioWin-clo/strontium/actions/workflows/test.yml)
[![Built on X1](https://img.shields.io/badge/Built%20on-X1-black)](https://x1.xyz)

> The green CI badge above means the code builds, passes linting, and clears the security audit on every commit.

🇬🇧 English | 🇵🇱 [Polski](README.pl.md)

X1 Strontium is a decentralized NTP time oracle for the [X1 blockchain](https://x1.xyz). It delivers cryptographically-attested UTC timestamps on-chain, sourced from a diverse mix of atomic clocks, commercial NTP providers, and community pools — verified by the validator network itself.

---

## The Problem

On Solana/X1, `Clock::unix_timestamp` is reported by the block leader — it can be manipulated by ±1–2 seconds without network-level detection. For most transactions this is irrelevant. But for:

- **Vesting contracts** — exact payout timing
- **Sub-second auctions** — who won?
- **Cross-chain time proofs** — verifiable across networks
- **Legal SLA contracts** — court-admissible timestamps

...leader-reported time is a serious vulnerability. X1 Strontium fixes this.

---

## How It Works

Each registered validator runs a lightweight Strontium daemon alongside Tachyon. Every **5 minutes** (configurable):

1. Queries all 21 NTP servers in parallel — atomic clocks, commercial providers, community pools from 4 continents
2. Selects the 5 best sources by tier (GPS/PPS → NTS → Stratum-1 → Pool) and RTT, deduplicating by resolved IP
3. Computes RTT-corrected median and validates spread (threshold: ±50ms)
4. Validates cross-tier consensus — at least 2 independent tiers must agree within ±60ms
5. Calculates a **confidence score**: `source_count × 0.4 + spread_quality × 0.4 + tier_weight × 0.2`
6. Checks the submitted timestamp against the on-chain clock — rejects if deviation exceeds 10 seconds
7. If confidence ≥ 0.60 → submits timestamp on-chain via `submit_time` + optional Memo Program
8. If sources disagree → **stays silent** (silence-as-signal = Byzantine fault protection)

Each submission includes a `sources_bitmap` so every round is fully auditable on-chain. The on-chain program aggregates submissions into a **288-slot ring buffer** via stake-weighted median. Manipulating the result requires compromising the majority of submitters simultaneously.

> **Why a mix of sources and not just government clocks?**
> The network is decentralized — we don't want to depend on a single country or institution.
> Each source is one vote. The median eliminates liars. More independent sources = stronger resistance.

---

## Architecture

```
Validator Server                          X1 Blockchain
┌──────────────────────────┐    ┌─────────────────────────────────┐
│  Tachyon Validator       │    │                                 │
│                          │    │  OracleState PDA                │
│  Strontium Daemon   ─TX─▶│    │  ┌───────────────────────────┐ │
│  ┌────────────────────┐  │    │  │  trusted_time_ms          │ │
│  │  NTP Autodiscovery │  │    │  │  spread_ms                │ │
│  │  ┌──────────────┐  │  │    │  │  confidence               │ │
│  │  │  GPS/PPS t-0 │  │  │    │  │  sources_bitmap           │ │
│  │  │  NTS      t-1│  │  │    │  │  ring_buffer[288]         │ │
│  │  │  Stratum1 t-2│  │  │    │  └───────────────────────────┘ │
│  │  │  Pool     t-3│  │  │    │                                 │
│  │  └──────────────┘  │  │    │  ValidatorRegistration PDA     │
│  │  Parallel queries  │  │    │  (TTL: 90 days, stake-checked) │
│  └────────────────────┘  │    │                                 │
└──────────────────────────┘    └─────────────────────────────────┘
```

Each transaction contains two instructions (memo optional):
- `submit_time` → writes to the on-chain ring buffer, outlier check against `Clock`
- `Memo Program` → human-readable log: `strontium:v1:w={window}:t={time}:c={confidence}:s={sources}`

---

## Requirements

| Requirement | Details |
|---|---|
| **OS** | Ubuntu 22.04 LTS or newer (GLIBC 2.35+) |
| **Solana CLI** | Installed and in PATH (`solana-keygen` must work) |
| **XNT balance** | ≥1 XNT on oracle keypair |
| **Self-stake** | ≥100 XNT verified on your validator |
| **Skip rate** | <10% (checked at registration) |
| **Network** | Port 123/UDP open outbound (NTP) |
| **Validator status** | Active on mainnet |

> **Check port 123 UDP:**
> ```bash
> nc -zu pool.ntp.org 123 && echo "OK — port open" || echo "BLOCKED — open with: sudo ufw allow out 123/udp"
> ```

> **Other Linux distributions:** Compile from source:
> ```bash
> git clone https://github.com/PioWin-clo/strontium
> cd strontium/daemon && cargo build --release
> ```

---

## Quick Start

### Step 1 — Download binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium
chmod +x strontium
x1sr help
```

### Step 2 — Generate oracle keypair

> ⚠️ **NEW dedicated keypair only.** Do NOT use `identity.json` or `vote.json`.
> If compromised, only the oracle keypair balance is at risk — your validator stays safe.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

### Step 3 — Fund oracle keypair

```bash
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json
# Send XNT to that address via XDEX, Backpack, CLI, or Ledger
```

Use the **[cost calculator](https://piowin-clo.github.io/strontium)** to choose the right amount for your interval and number of operators.

### Step 4 — Register

> ⚠️ `vote.json` is your validator's vote keypair — lives on the server at `~/.config/solana/vote.json`. NOT your Ledger withdraw key.

```bash
x1sr register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Registration validates: validator active, skip rate <10%, self-stake ≥100 XNT.

> Registration expires after **90 days** — re-register before expiry with `x1sr register`.

### Step 5 — Start daemon

**Dry-run** (NTP consensus only, no on-chain transactions, zero cost):

```bash
x1sr start --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Live mode** (submits every 5 minutes):

```bash
nohup x1sr start \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

```bash
x1sr status
tail -f ~/strontium.log   # You should see: ✅ submit OK — tx: ...
```

### Step 6 — Install as system service

```bash
sudo x1sr install
```

Automatically detects username and binary path, checks balance, generates and enables `/etc/systemd/system/strontium.service`. The service waits 2 minutes after boot before starting (gives Tachyon time to join the network).

---

## CLI Reference

```
x1sr start              Start daemon (live mode)
x1sr start --dry-run    Start in test mode (no transactions)
x1sr stop               Stop daemon
x1sr status             Status, NTP consensus, balance, rotation
x1sr sources            NTP sources table (RTT, offset, tier, NTS)
x1sr history [N]        Last N on-chain submissions (default: 10)
x1sr register           Register validator oracle
x1sr deregister         Deregister (coming soon)
x1sr balance            Oracle keypair balance and runway
x1sr archive            Export on-chain history to JSONL
x1sr config show        Show current configuration
x1sr config set K V     Set a configuration value
x1sr install            Install as systemd service (run with sudo)
x1sr uninstall          Remove systemd service
```

**Configuration keys** (`x1sr config set <key> <value>`):

| Key | Default | Description |
|---|---|---|
| `interval` | `300` | Submit interval in seconds |
| `keypair` | `~/.config/strontium/oracle-keypair.json` | Oracle keypair path |
| `vote_keypair` | auto-detect | Vote keypair path |
| `rpc` | localhost + mainnet | Add RPC endpoint (prepended to list) |
| `committee` | *(empty = solo)* | Add oracle pubkey to rotation committee |
| `committee_clear` | — | Clear committee list |
| `dry_run` | `false` | Test mode (true/false) |
| `memo` | `true` | Include Memo Program in TX (false = lower compute units) |

---

## Rotation — Sharing the Cost

Multiple validators can coordinate submissions to share costs and improve coverage. The daemon uses deterministic round-robin rotation — **no communication between servers needed**:

```
window_id = current_time / interval_s
primary   = window_id % committee_size
```

Every daemon independently calculates whose turn it is. A faster server or better connection gives no advantage — the result is the same for everyone.

**Staged fallback** (prevents gaps if primary is offline):

- `t + 0s` → primary submits
- `t + 30s` → backup-1 submits if primary was silent
- `t + 60s` → backup-2 submits if still silent

**How to configure rotation:**

```bash
# Add both oracle pubkeys to the committee (run on both servers)
x1sr config set committee <PRIME_ORACLE_PUBKEY>
x1sr config set committee <SENTINEL_ORACLE_PUBKEY>

# Verify
x1sr config show
```

The list is automatically sorted — the order you add them doesn't matter. Restart the daemon after changes.

---

## Cost and Accuracy

Each transaction costs **0.002 XNT** (verified on-chain). Use the **[interactive cost calculator](https://piowin-clo.github.io/strontium)** to model your exact setup.

Quick reference (cost per operator):

| Operators | Interval | TX/day/op | XNT/month/op | On-chain accuracy |
|---|---|---|---|---|
| 1 (solo) | 300s | 288 | ~17.3 XNT | ±3–10 ms |
| 2 (committee) | 300s | 144 | ~8.6 XNT | ±2–6 ms |
| 5 (committee) | 300s | 58 | ~3.5 XNT | ±2–6 ms |
| 10 (committee) | 300s | 29 | ~1.7 XNT | ±2–5 ms |
| 50 (committee) | 300s | 6 | ~0.35 XNT | ±1–4 ms |
| any + GPS/PPS | any | — | — | ±50 nanoseconds |

> When XNT price rises, the right response is more operators sharing the cost — not degrading the service by increasing the interval.

Change interval:

```bash
x1sr config set interval 600    # every 10 minutes
x1sr config set interval 3600   # every hour
```

---

## NTP Sources (21 total)

| Tier | Source | Type | Region |
|---|---|---|---|
| **T-0 GPS** | `/dev/pps0` | GPS/PPS hardware | Local server |
| **T-1 NTS** | `ptbtime1.ptb.de` | Atomic + NTS auth | Germany |
| **T-1 NTS** | `time.cloudflare.com` | Commercial + NTS auth | Global |
| **T-1 NTS** | `ntp.time.nl` | Atomic + NTS auth | Netherlands |
| **T-2 S1** | `nts.netnod.se` | Atomic Stratum-1 | Sweden |
| **T-2 S1** | `ptbtime2.ptb.de` | Government atomic | Germany |
| **T-2 S1** | `ptbtime3.ptb.de` | Government atomic | Germany |
| **T-2 S1** | `tempus1.gum.gov.pl` | Government atomic | Poland |
| **T-2 S1** | `tempus2.gum.gov.pl` | Government atomic | Poland |
| **T-2 S1** | `tempus3.gum.gov.pl` | Government atomic | Poland |
| **T-2 S1** | `nist1-atl.ustiming.org` | Government atomic | USA |
| **T-2 S1** | `time.nist.gov` | Government atomic | USA |
| **T-2 S1** | `ntp.jst.mfeed.ad.jp` | Government atomic | Japan |
| **T-2 S1** | `syrte.obspm.fr` | Government atomic | France |
| **T-2 S1** | `ntp-p1.obspm.fr` | Government atomic | France |
| **T-2 S1** | `ntp.metas.ch` | Government atomic | Switzerland |
| **T-2 S1** | `time.google.com` | Commercial | Global |
| **T-2 S1** | `ntp.nic.cz` | Government Stratum-1 | Czech Republic |
| **T-2 S1** | `ntp1.fau.de` | University atomic | Germany |
| **T-3 Pool** | `0.pool.ntp.org` | Community | Global |
| **T-3 Pool** | `1.pool.ntp.org` | Community | Global |
| **T-3 Pool** | `europe.pool.ntp.org` | Community | Europe |

All sources queried in parallel. List refreshed every hour. Sources are deduplicated by resolved IP (anycast pool protection). The daemon selects the 5 best sources per cycle by tier priority, then RTT, requiring at least 3 Stratum-1 or better.

**GPS/PPS (optional):** The daemon auto-detects `/dev/pps0` at startup. If present, GPS/PPS is used as tier-0 (±50ns accuracy) with NTP as cross-check. If absent, falls back to NTP automatically — **no configuration needed, no errors**. Recommended hardware: u-blox NEO-M8N (~$30 USB).

---

## On-Chain Addresses

| | Address |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [View on X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Reading Time On-Chain

Every submission is visible in the explorer. Each transaction contains a Memo:

```
strontium:v1:w=1234:t=1712780400000:c=87:s=5
```

where: `w` = window id, `t` = Unix time in ms, `c` = confidence (0–100), `s` = sources used.

All submissions: [X1 Explorer — Oracle PDA](https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn)

For on-chain integration via Anchor, read the `OracleState` account at the Oracle PDA address and use `latest_trusted_time_ms`. Check `staleness_slots` against your maximum acceptable staleness before trusting the value.

---

## Troubleshooting

**Daemon silent for many cycles:**

```bash
x1sr status    # check silent_reason field
x1sr sources   # check which NTP servers respond
```

| Silent reason | What to do |
|---|---|
| `no_valid_sources` | Check port 123/UDP: `nc -zu pool.ntp.org 123` |
| `spread_too_high` | NTP sources disagree by >50ms — wait or check connectivity |
| `low_confidence` | Not enough quality sources — check `x1sr sources` |
| `not_elected` | Rotation: another validator's window — normal |
| `registration_expired` | Run `x1sr register` again (TTL 90 days) |
| `insufficient_balance` | Fund oracle keypair — check `x1sr balance` |
| `dry_run` | Test mode active — restart without `--dry-run` |
| `timestamp_outlier` | NTP time deviates >10s from chain clock — check chrony |

**Registration errors:**

| Error | Solution |
|---|---|
| `AccountNotFound` | Fund oracle keypair (Step 3) |
| `AccountNotSigner` | Check `--vote-keypair` path |
| `Insufficient self-stake` | Increase self-stake to ≥100 XNT via XDEX Valistake |
| `Skip rate too high` | Wait for validator skip rate to drop below 10% |

**Binary won't run (`GLIBC not found`):**

```bash
git clone https://github.com/PioWin-clo/strontium
cd strontium/daemon && cargo build --release
target/release/strontium help
```

---

## Security

**Upgrade authority:** `7k4tvn5Aim8yWEdSAfZqptTvTf7r1WXUNSNa8evmmNGq` (Ledger — cold storage)

Program upgrades require physical Ledger confirmation. The oracle fee-payer key (`EgFaM42n...`) has no upgrade capability.

| Attack | Mitigation |
|---|---|
| Single validator lying | On-chain outlier check: rejected if timestamp deviates >10s from `Clock` |
| Coordinated timestamp manipulation | Stake-weighted median — requires majority of submitters |
| NTP MITM | Multi-continental cross-check (50ms threshold) + cross-tier validation |
| Submission spam | ValidatorRegistration required (vote proof + stake check) |
| Oracle key compromise | Only oracle keypair exposed — identity/vote/upgrade authority untouched |
| GPS spoofing | Cross-checked against NTP consensus (±5s threshold) |
| Program upgrade attack | Upgrade authority on cold Ledger — no hot key can upgrade |

**Responsible disclosure:** [GitHub Issues](https://github.com/PioWin-clo/strontium/issues) or X1 Validator Army Telegram.

---

## Pre-Mainnet Checklist

Before running in production with a live committee:

- [ ] Oracle keypair funded (`x1sr balance` — at least 30 days runway)
- [ ] Registration confirmed (`x1sr status` shows `running`)
- [ ] Dry-run completed successfully for at least 3 cycles
- [ ] NTP sources responding (`x1sr sources` shows ≥3 active)
- [ ] Port 123/UDP open outbound
- [ ] Committee configured on all nodes if running rotation
- [ ] Failover tested: stop primary, verify backup submits within 60s
- [ ] systemd service installed (`sudo x1sr install`)

---

## Roadmap

- [x] Parallel NTP querying with 4-tier source classification (21 servers, 4 continents)
- [x] On-chain ring buffer (288 slots, `zero_copy`)
- [x] ValidatorRegistration — vote account proof + stake check + TTL 90d
- [x] `sources_bitmap` per submission — full auditability
- [x] Confidence scoring
- [x] Full CLI (`start`, `stop`, `status`, `sources`, `config`, `install`, ...)
- [x] Automatic systemd installer
- [x] Memo Program in every transaction — full transparency (optional via config)
- [x] Circuit breaker RPC with exponential backoff
- [x] Deterministic round-robin rotation (`slot % n`) — fair cost sharing
- [x] `ed25519-dalek` v2, clean Clippy, security audit
- [x] Cross-tier consensus validation (at least 2 independent tiers must agree)
- [x] IP deduplication — pool anycast protection
- [x] On-chain outlier slashing — reject submissions >10s from `Clock`
- [x] Upgrade authority on cold Ledger storage
- [x] Interactive cost calculator — [piowin-clo.github.io/strontium](https://piowin-clo.github.io/strontium)
- [ ] GPS/PPS production-tested path (hardware required)
- [ ] Full NTS client-side protocol (cryptographic handshake)
- [ ] Dashboard — consensus visualization, history, validator health
- [ ] Alpenglow integration (τₖ phase-lock — the missing time layer for eigenvm)

---

## Built on X1

X1 Strontium is open-source infrastructure for the X1 ecosystem. Built with Anchor 0.31.1 on Tachyon 2.2.20. CI: Build + Clippy + Security audit on every commit.

**Standing on open shoulders:** X1 Strontium was conceived independently, but could not exist without Jack Levin's vision and the work of the entire X1 team — Photon Oracle, Entropy Engine, and the X1 blockchain itself. Jack and his team built the foundation. We built on it.

**Concept & architecture:** PioWin
**Code:** Claude (Anthropic) with support from Theo (Cyberdyne)
