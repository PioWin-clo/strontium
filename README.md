# ⚛️ X1 Strontium

**Atomic-grade time for blockchain.**

X1 Strontium is a decentralized NTP time oracle for the [X1 blockchain](https://x1.xyz). It provides cryptographically-attested UTC timestamps on-chain, sourced from government atomic clocks — verified by the validator network itself.

---

## The Problem

Every blockchain has a time problem. On Solana/X1, `Clock::unix_timestamp` is reported by the slot leader — it can be manipulated by ±1-2 seconds without detection. For most transactions this doesn't matter. But for:

- **Vesting contracts** — exact payout timing
- **Sub-second auctions** — who won?
- **Cross-chain time proofs** — verifiable across networks
- **Legal SLA contracts** — court-admissible timestamps

...leader-reported time is a serious vulnerability.

## The Solution

Each validator runs a lightweight Strontium daemon alongside Tachyon. Every 60 seconds it:

1. Queries 5 atomic NTP servers from 3+ continents (PTB Germany, GUM Poland, NIST USA, NICT Japan + NTP Pool)
2. Computes RTT-corrected offsets and checks spread (±20ms threshold)
3. If consensus is reached → submits timestamp on-chain
4. If sources disagree → **stays silent** (silence-as-signal)

The on-chain program aggregates submissions via median. To manipulate the result you'd need to simultaneously compromise atomic clock laboratories on 3 continents.

---

## Architecture

```
Validator Server                    X1 Blockchain
┌─────────────────────┐            ┌──────────────────────┐
│  Tachyon Validator  │            │                      │
│                     │            │  OracleState PDA     │
│  Strontium Daemon   │───submit──▶│  ┌────────────────┐  │
│  ┌───────────────┐  │            │  │ trusted_time   │  │
│  │ NTP Consensus │  │            │  │ spread_ms      │  │
│  │ PTB Germany   │  │            │  │ confidence     │  │
│  │ GUM Poland    │  │            │  │ ring_buffer    │  │
│  │ NIST USA      │  │            │  │ [256 entries]  │  │
│  │ NICT Japan    │  │            │  └────────────────┘  │
│  │ NTP Pool      │  │            │                      │
│  └───────────────┘  │            └──────────────────────┘
└─────────────────────┘
```

**Key properties:**
- Zero trust: no authority can modify the median
- Silence-as-signal: Byzantine fault tolerance built-in
- Permissionless: any active X1 validator can join

---

## Requirements

- **OS:** Ubuntu 22.04 LTS (binary requires GLIBC 2.35+)
- **Solana CLI:** installed and in PATH (`solana-keygen` must work)
- **XNT balance:** ~1 XNT on oracle keypair (for registration + ~138 days of submissions)
- **Network:** ports 123/UDP open outbound (NTP)

---

## Quick Start

### Step 1 — Download binary

```bash
wget https://github.com/PioWin-clo/strontium/releases/latest/download/strontium
chmod +x strontium
./strontium --help
```

### Step 2 — Generate oracle keypair

> ⚠️ **Important:** This is a NEW dedicated keypair — do NOT use your `identity.json` or `vote.json`.

```bash
mkdir -p ~/.config/strontium
solana-keygen new \
  --outfile ~/.config/strontium/oracle-keypair.json \
  --no-bip39-passphrase
chmod 600 ~/.config/strontium/oracle-keypair.json
```

Note the pubkey shown — you'll need to fund it in the next step.

### Step 3 — Fund oracle keypair

The oracle keypair pays for registration and ongoing submissions (~0.216 XNT/month). Send at least **1 XNT**:

```bash
# Check the oracle keypair address
solana-keygen pubkey ~/.config/strontium/oracle-keypair.json

# Send 1 XNT from your main wallet (replace SOURCE_KEYPAIR with your wallet)
solana transfer \
  <ORACLE_PUBKEY> \
  1 \
  --url https://rpc.mainnet.x1.xyz \
  --keypair <SOURCE_KEYPAIR> \
  --allow-unfunded-recipient
```

### Step 4 — Register

> ⚠️ **Important:** `vote.json` is your validator's vote keypair — it lives on your server at `~/.config/solana/vote.json`. This is NOT the Ledger withdraw key.

```bash
./strontium register \
  --keypair ~/.config/strontium/oracle-keypair.json \
  --vote-keypair ~/.config/solana/vote.json
```

Expected output:
```
✓ Registration successful!
  TX: <signature>
  Explorer: https://explorer.mainnet.x1.xyz/tx/<signature>
```

### Step 5 — Run daemon

**Test mode (no transactions sent):**
```bash
./strontium --keypair ~/.config/strontium/oracle-keypair.json --dry-run
```

**Live mode (background):**
```bash
nohup ./strontium \
  --keypair ~/.config/strontium/oracle-keypair.json \
  > ~/strontium.log 2>&1 &
echo "Strontium PID: $!"
```

**Check it's working:**
```bash
tail -f ~/strontium.log
```

You should see `✅ submit OK — tx: ...` every 60 seconds.

---

## Troubleshooting

### `GLIBC_2.39 not found`
Your system is too old. Strontium requires Ubuntu 22.04+ (GLIBC 2.35+). Ubuntu 20.04 is not supported.

### `AccountNotFound` during registration
Oracle keypair has no XNT. See Step 3 — fund the keypair first.

### `AccountNotSigner` during registration
Make sure you're using the correct `--vote-keypair` path. On most validators this is `~/.config/solana/vote.json`.

### `Transaction signature verification failure`
The oracle keypair file may be corrupted. Generate a new one (Step 2) and fund it again.

### Daemon silent for many cycles
```bash
tail -20 ~/strontium.log
```
If spread or confidence is low, the daemon is correctly staying silent (Byzantine fault protection). Check your server's NTP connectivity: `ntpdate -q pool.ntp.org`

---

## NTP Sources

| Source | Type | Stratum | Location |
|---|---|---|---|
| PTB Germany | Government atomic | 1 | Brunswick, DE |
| GUM Poland | Government atomic | 1 | Warsaw, PL |
| SYRTE France | Government atomic | 1 | Paris, FR |
| METAS Switzerland | Government atomic | 1 | Bern, CH |
| Netnod Sweden | Government atomic | 1 | Stockholm, SE |
| SIDN Netherlands | Government | 1 | Arnhem, NL |
| NIST USA | Government atomic | 1 | Boulder, CO |
| USNO USA | Government atomic | 1 | Washington, DC |
| NICT Japan | Government atomic | 1 | Tokyo, JP |
| CAS China | Government atomic | 1 | Beijing, CN |
| NTP Pool (global) | Open-source community | 2-3 | Global |
| NTP Pool (Europe) | Open-source community | 2-3 | Europe |
| NTP Pool (Americas) | Open-source community | 2-3 | Americas |
| NTP Pool (Asia) | Open-source community | 2-3 | Asia |
| Cloudflare | Commercial | 3 | Global |
| Google | Commercial | 3 | Global |

The daemon selects the 5 lowest-latency servers from at least 2 continents. Stratum 1 (government atomic) sources are preferred.

---

## On-Chain Addresses

| | Address |
|---|---|
| **Program ID** | `2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe` |
| **Oracle PDA** | `EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn` |
| **Explorer** | [View on X1 Explorer](https://explorer.mainnet.x1.xyz/address/2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe) |

---

## Reading Time On-Chain

```rust
// In your Anchor program:
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct YourInstruction<'info> {
    #[account(
        address = "EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn".parse().unwrap()
    )]
    pub oracle: AccountInfo<'info>,
}

// Get trusted UTC timestamp (milliseconds):
let trusted_time_ms = strontium::cpi::read_time(
    CpiContext::new(strontium_program, accounts),
    max_staleness_slots
)?;
```

---

## Accuracy

| Active validators | Accuracy |
|---|---|
| 1 | ±3-10ms |
| 5 | ±2-6ms |
| 10 | ±2-5ms |
| 50+ | ±1-4ms |

Physical limit: NTP network latency (~1-5ms). Future improvement: GPS/PPS modules → ±50 nanoseconds.

---

## Operating Costs

| Per validator | Cost |
|---|---|
| Daily | ~0.0072 XNT |
| Monthly | ~0.216 XNT (~$0.09 at current prices) |

Cost scales with XNT price. At $1/XNT → ~$0.22/month. At $10/XNT → ~$2.16/month.

---

## Security

**Upgrade authority:** Currently held by the project author (`EgFaM42nFeZYwDXzMZWNTmp5ojyL7UGP8xgdX1SBXYsb`). Transfer to community multisig is planned as the network of submitters grows.

**Threat model:**
- Single validator lying → eliminated by median (requires >50% to move it)
- NTP MITM attack → cross-continental check detects divergence
- Spam submissions → ValidatorRegistration required (vote account proof)
- Keypair compromise → deregister + re-register; only ~0.22 XNT at risk

**Responsible disclosure:** Open a [GitHub issue](https://github.com/PioWin-clo/strontium/issues) or contact via X1 Validators Telegram.

---

## Roadmap

- [x] Core NTP consensus daemon
- [x] On-chain median aggregation (zero_copy, ring buffer)
- [x] ValidatorRegistration (vote account proof)
- [ ] Stake threshold enforcement (MIN_STAKE > 0)
- [ ] Registration TTL (auto-expire inactive submitters)
- [ ] GPS/PPS support for sub-millisecond accuracy
- [ ] Roughtime support (cryptographic time authentication)
- [ ] Alpenglow integration (time attestation layer)
- [ ] Community multisig upgrade authority

---

## Built on X1

X1 Strontium is open-source infrastructure for the X1 ecosystem. It uses [Anchor](https://anchor-lang.com) 0.31.1 on [Tachyon](https://x1.xyz) 2.2.20.

*"Strontium" — named after the element used in the world's most accurate optical atomic clocks, more precise than caesium-based UTC.*

---

## Auto-start on Boot (systemd)

To run Strontium automatically when the server starts:

**Step 1 — Create service file:**

```bash
sudo nano /etc/systemd/system/strontium.service
```

Paste this content (replace `x1pio` with your username if different):

```ini
[Unit]
Description=X1 Strontium Time Oracle Daemon
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=x1pio
ExecStart=/home/x1pio/strontium --keypair /home/x1pio/.config/strontium/oracle-keypair.json
Restart=on-failure
RestartSec=10
StandardOutput=append:/home/x1pio/strontium.log
StandardError=append:/home/x1pio/strontium.log

[Install]
WantedBy=multi-user.target
```

**Step 2 — Enable and start:**

```bash
sudo systemctl daemon-reload
sudo systemctl enable strontium
sudo systemctl start strontium
```

**Step 3 — Check status:**

```bash
sudo systemctl status strontium
tail -f ~/strontium.log
```

**Useful commands:**

```bash
sudo systemctl stop strontium      # stop daemon
sudo systemctl restart strontium   # restart daemon
sudo systemctl disable strontium   # remove from autostart
journalctl -u strontium -f         # view systemd logs
```
