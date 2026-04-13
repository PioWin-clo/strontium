# X1 Strontium ⏱

**Decentralized atomic time oracle for X1 blockchain**

X1 Strontium provides certified UTC time on-chain by aggregating measurements from 45+ Stratum-1 NTP servers across 4 continents. Validator operators run a lightweight daemon that submits consensus timestamps to an Anchor smart contract, building a tamper-resistant time reference that any X1 program can read.

> **Why it matters:** X1's `Clock::unix_timestamp` is reported by block leaders and can drift by 14–56 seconds. Strontium provides independently verified time from atomic clock sources — the missing infrastructure layer for time-sensitive contracts.

---

## Architecture

```
GPS/PPS (tier-0) ──┐
NTS servers  (tier-1) ──┤
Stratum-1    (tier-2) ──┼─→ NTP consensus ──→ submit_time TX ──→ OracleState PDA
Pool servers (tier-3) ──┘    (median, IQR filter,              (ring buffer 288 slots,
                               cross-tier validation)            24h of history)
```

**Key design decisions:**
- 45 NTP servers across Europe, Americas, Asia-Pacific — geographic diversity reduces manipulation surface
- IQR outlier filter removes spiking servers before spread calculation
- Cross-tier validation requires ≥1 Stratum-1/NTS source to agree with median
- GPS/PPS (if available) bypasses cross-tier requirement and provides ±50ns accuracy
- Slot-hash based rotation with staged fallback — deterministic, unpredictable without controlling block production
- `sources_bitmap` (u32) provides full on-chain audit trail of which servers contributed
- 90-day registration TTL with 7-day renewal window
- Silence-as-signal: daemon stays silent when sources disagree or spread exceeds 50ms

---

## Quick Start

### Prerequisites
- X1 validator node running (Tachyon v2.2.20+)
- Ubuntu 22.04
- ~5 XNT on oracle keypair for ~9 days of operation

### Install

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium-linux-x86_64 -O strontium
chmod +x strontium
sudo mv strontium /usr/local/bin/strontium
sudo ln -sf /usr/local/bin/strontium /usr/local/bin/x1sr
```

### Setup

```bash
# 1. Generate oracle keypair (dedicated — NOT your identity.json)
mkdir -p ~/.config/strontium
solana-keygen new --outfile ~/.config/strontium/oracle-keypair.json --no-bip39-passphrase

# 2. Fund the oracle keypair (min 1 XNT, recommended 5 XNT)
solana transfer $(solana-keygen pubkey ~/.config/strontium/oracle-keypair.json) 5 \
  --url https://rpc.mainnet.x1.xyz --keypair ~/.config/solana/id.json

# 3. Register on-chain
x1sr register

# 4. Start daemon
x1sr start
```

### Install as systemd service (auto-start on boot)

```bash
sudo x1sr install
```

---

## Configuration

```bash
x1sr config show                          # show current config
x1sr config set interval 300             # submit interval (seconds)
x1sr config set rpc https://rpc.mainnet.x1.xyz
x1sr config set alert_webhook https://hooks.slack.com/...   # Telegram/Discord/Slack
x1sr config set alert_balance 1.0        # alert threshold (XNT)
x1sr config set tier_threshold 60        # cross-tier consensus tolerance (ms)
x1sr config set dry_run true             # test mode (no transactions)
```

---

## Commands

| Command | Description |
|---|---|
| `x1sr start` | Start daemon |
| `x1sr start --dry-run` | Test mode — NTP consensus without sending TX |
| `x1sr stop` | Stop daemon |
| `x1sr status` | Show status, NTP consensus, balance |
| `x1sr sources` | Show NTP source details |
| `x1sr register` | Register on-chain (one-time) |
| `x1sr deregister` | Close registration PDA, recover rent |
| `x1sr balance` | Check oracle keypair balance and runway |
| `x1sr history [n]` | Show last N on-chain submissions |
| `x1sr config show` | Show configuration |
| `x1sr config set <key> <value>` | Set config value |
| `x1sr install` | Install as systemd service |
| `x1sr uninstall` | Remove systemd service |

---

## Status Output

```
X1 Strontium — daemon status
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Daemon         : running (PID 12345)
  Oracle keypair : 4o6xfpT1LWPB6Um4f6oPeZZdSeUZoUafhckpbqXFpYcQ
  Balance        : 4.998 XNT (~9 days) [OK]
  Last submit    : 20:29:51.862 UTC
  Interval       : 300s
  Mode           : live
  NTP consensus  : 20:29:51.862 UTC
  Spread         : 18ms
  Confidence     : 0.82
```

---

## On-Chain Memo Format

Every submission includes a human-readable Memo:

```
strontium:v1:w=1234:ntp=20:29:51.8621:chain=20:29:37.0000:c=82:s=7:st=1
```

| Field | Description |
|---|---|
| `w=` | Window ID (unix_time / interval) |
| `ntp=` | NTP consensus time HH:MM:SS.mmmm |
| `chain=` | X1 chain clock HH:MM:SS (shows drift) |
| `c=` | Confidence 0-100 |
| `s=` | Active NTP sources |
| `st=` | Best stratum (1 = atomic clock) |

---

## Reading Time from Smart Contracts

```rust
use anchor_lang::prelude::*;

// CPI call to Strontium
let time = strontium::cpi::read_time(
    ctx.accounts.strontium_ctx(),
    300, // max_staleness_slots
)?;

require!(time.confidence_pct >= 80, MyError::TimeTrustTooLow);
require!(time.spread_ms <= 30,       MyError::TimeSpreadTooHigh);
require!(time.staleness_slots <= 200, MyError::TimeStale);

let now = time.timestamp_ms;
```

**TimeReading struct:**

```rust
pub struct TimeReading {
    pub timestamp_ms:    i64,   // UTC milliseconds
    pub confidence_pct:  u8,    // 0-100
    pub spread_ms:       i16,   // inter-source spread
    pub sources_count:   u8,    // active submitters
    pub staleness_slots: u64,   // slots since last update
}
```

---

## Program IDs (Mainnet)

| Account | Address |
|---|---|
| Program | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| Oracle PDA | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |

---

## NTP Sources

45 servers across 4 tiers and 4 continents:

| Tier | Count | Examples |
|---|---|---|
| T-0 GPS/PPS | auto-detect | `/dev/pps0` |
| T-1 NTS | 4 | ptbtime1.ptb.de, time.cloudflare.com |
| T-2 Stratum-1 | 28 | tempus1.gum.gov.pl, ntp.metas.ch, time.nist.gov, ntp.nict.jp |
| T-3 Pool | 6 | pool.ntp.org, europe/asia/north-america zones |

> **Note:** NTS (Network Time Security) tier is planned for v1.5. Currently queried via plain NTP. Authentication will be added with rustls in a future release.

---

## Economics

| Parameter | Value |
|---|---|
| Cost per TX (submit + memo) | ~0.002 XNT |
| TXs per day (300s interval) | ~288 |
| Daily cost | ~0.576 XNT |
| 5 XNT runway | ~8.7 days |
| Registration cost | ~0.002 XNT (rent) |
| Registration TTL | 90 days |

Use the [cost calculator](https://piowin-clo.github.io/strontium) to estimate your runway.

---

## Roadmap

- [ ] NTS authentication (rustls/nts-rs) — T-1 tier upgrade
- [ ] solana_sdk + anchor-client TX construction — IDL-based
- [ ] TypeScript integration tests — CI end-to-end
- [ ] GPS u-blox $50 hardware integration
- [ ] Prometheus metrics endpoint
- [ ] Alpenglow protocol layer integration

---

## Security

- Use a **dedicated oracle keypair** — never use `identity.json`
- The oracle keypair only needs enough XNT for transaction fees
- If compromised, the blast radius is limited to fee funds
- Registration PDA is tied to both oracle keypair AND vote account

---

## License

MIT — built by [Piotr "Killer" Winkler](https://github.com/PioWin-clo)
