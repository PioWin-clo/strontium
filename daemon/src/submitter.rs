// P1: Corrected Solana PDA derivation
// P3: TODO — migrate to solana_sdk + anchor-client (tracked issue)
// P8: chain time used only for Memo, not for blocking TX

use sha2::{Sha256, Digest};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::consensus::ConsensusResult;
use crate::ntp_client::NtpSource;

// ── Constants ────────────────────────────────────────────────────────────────

const MEMO_PROGRAM_ID: [u8; 32] = [
    0x05, 0x4a, 0x53, 0x5a, 0x99, 0x29, 0x21, 0x06,
    0x4d, 0x24, 0xe8, 0x71, 0x60, 0xda, 0x38, 0x7c,
    0x7c, 0x35, 0xb5, 0xdd, 0xbc, 0x92, 0xbb, 0x81,
    0xe4, 0x1f, 0xa8, 0x40, 0x41, 0x05, 0x44, 0x8d,
];

// ── PDA Derivation (P1: Correct Solana algorithm) ────────────────────────────

/// Check if a 32-byte value is on the ed25519 curve
/// A valid PDA must be OFF the curve (not a valid public key)
fn is_on_curve(bytes: &[u8; 32]) -> bool {
    use curve25519_dalek::edwards::CompressedEdwardsY;
    CompressedEdwardsY(*bytes).decompress().is_some()
}

/// Derive a Program Derived Address using the correct Solana algorithm:
/// SHA256(seeds... || nonce || program_id || "ProgramDerivedAddress")
/// Iterate nonce from 255 down to 0 until result is off-curve
pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> ([u8; 32], u8) {
    for nonce in (0u8..=255).rev() {
        let mut h = Sha256::new();
        for seed in seeds {
            h.update(seed);
        }
        h.update([nonce]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let hash: [u8; 32] = h.finalize().into();
        if !is_on_curve(&hash) {
            return (hash, nonce);
        }
    }
    panic!("Could not find valid program address for given seeds");
}

/// Derive Registration PDA: seeds = [b"reg", oracle_pubkey]
pub fn derive_registration_pda(oracle_pubkey: &[u8; 32], program_id: &[u8; 32]) -> [u8; 32] {
    let (pda, _bump) = find_program_address(&[b"reg", oracle_pubkey.as_ref()], program_id);
    pda
}

/// Derive Oracle State PDA: seeds = [b"strontium"]
pub fn derive_oracle_state_pda(program_id: &[u8; 32]) -> [u8; 32] {
    let (pda, _bump) = find_program_address(&[b"strontium"], program_id);
    pda
}

// ── RPC Client ───────────────────────────────────────────────────────────────

pub struct RpcClient {
    urls:           Vec<String>,
    fail_counts:    Vec<u32>,
    cooldown_until: Vec<u64>,
}

impl RpcClient {
    pub fn new(urls: Vec<String>) -> Self {
        let n = urls.len();
        Self { urls, fail_counts: vec![0; n], cooldown_until: vec![0; n] }
    }

    fn rpc_call_with_retry<F, T>(&mut self, payload: &str, parse: F) -> Result<T, String>
    where F: Fn(&str) -> Result<T, String>,
    {
        let now = unix_secs();
        for i in 0..self.urls.len() {
            if self.cooldown_until[i] > now { continue; }
            match self.do_call(&self.urls[i].clone(), payload) {
                Ok(resp) => match parse(&resp) {
                    Ok(val) => {
                        self.fail_counts[i] = 0;
                        return Ok(val);
                    }
                    Err(e) => {
                        self.fail_counts[i] += 1;
                        if self.fail_counts[i] >= 3 {
                            self.cooldown_until[i] = now + 300;
                        }
                        eprintln!("[rpc] Parse error on {}: {}", self.urls[i], e);
                    }
                },
                Err(e) => {
                    self.fail_counts[i] += 1;
                    if self.fail_counts[i] >= 3 {
                        self.cooldown_until[i] = now + 300;
                    }
                    eprintln!("[rpc] Call error on {}: {}", self.urls[i], e);
                }
            }
        }
        Err("No healthy RPC endpoint available".to_string())
    }

    fn do_call(&self, url: &str, payload: &str) -> Result<String, String> {
        ureq::post(url)
            .set("Content-Type", "application/json")
            .set("User-Agent", "X1-Strontium/1.0 (https://github.com/PioWin-clo/strontium)")
            .timeout(std::time::Duration::from_secs(10))
            .send_string(payload)
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }

    pub fn get_recent_blockhash(&mut self) -> Result<[u8; 32], String> {
        let payload = r#"{"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[{"commitment":"finalized"}]}"#;
        self.rpc_call_with_retry(payload, |resp| parse_blockhash(resp))
    }

    pub fn get_balance(&mut self, pubkey: &str) -> Result<u64, String> {
        let payload = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"getBalance","params":["{}", {{"commitment":"confirmed"}}]}}"#,
            pubkey
        );
        self.rpc_call_with_retry(&payload, |resp| parse_balance(resp))
    }

    pub fn send_transaction(&mut self, tx_b64: &str) -> Result<String, String> {
        let payload = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":["{}",{{"encoding":"base64","preflightCommitment":"confirmed"}}]}}"#,
            tx_b64
        );
        self.rpc_call_with_retry(&payload, |resp| parse_signature(resp))
    }

    // P8: chain time only for Memo — single call using getSlot + getBlockTime
    pub fn get_chain_time_ms(&mut self) -> Option<i64> {
        let slot_payload = r#"{"jsonrpc":"2.0","id":1,"method":"getSlot"}"#;
        let slot = self.rpc_call_with_retry(slot_payload, |resp| {
            let prefix = "\"result\":";
            let start = resp.find(prefix).ok_or("no result")?;
            let rest = resp[start + prefix.len()..].trim_start();
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse::<u64>().map_err(|e| e.to_string())
        }).ok()?;

        let bt_payload = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"getBlockTime","params":[{}]}}"#, slot
        );
        let ts_s = self.rpc_call_with_retry(&bt_payload, |resp| {
            let prefix = "\"result\":";
            let start = resp.find(prefix).ok_or("no result")?;
            let rest = resp[start + prefix.len()..].trim_start();
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse::<i64>().map_err(|e| e.to_string())
        }).ok()?;

        Some(ts_s * 1000)
    }
}

// ── Submit parameters ─────────────────────────────────────────────────────────

pub struct SubmitParams<'a> {
    pub consensus:     &'a ConsensusResult,
    pub window_id:     u64,
    pub memo_enabled:  bool,
    pub chain_time_ms: Option<i64>,  // P8: for Memo only
}

// ── Transaction builder ───────────────────────────────────────────────────────

use ed25519_dalek::SigningKey;

}

fn anchor_discriminator(name: &str) -> [u8; 8] {
    let mut h = Sha256::new();
    h.update(format!("global:{}", name).as_bytes());
    let hash: [u8; 32] = h.finalize().into();
    hash[..8].try_into().unwrap()
}

fn encode_compact_u16(n: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut val = n;
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if val != 0 { byte |= 0x80; }
        out.push(byte);
        if val == 0 { break; }
    }
    out
}

    program_id: &[u8; 32],
    oracle_pda: &[u8; 32],
    reg_pda:    &[u8; 32],
    blockhash:  &[u8; 32],
    ix_data:    &[u8],
) -> Vec<u8> {
    let accounts = [oracle, oracle_pda, reg_pda, program_id];
    let num_accounts = accounts.len() as u8;

    let mut msg = Vec::new();
    // Header: num_required_signatures=1, num_readonly_signed=0, num_readonly_unsigned=1
    msg.extend_from_slice(&[1u8, 0u8, 1u8]);
    // Account list
    msg.extend_from_slice(&encode_compact_u16(num_accounts as u16));
    for acc in &accounts { msg.extend_from_slice(*acc); }
    // Blockhash
    msg.extend_from_slice(blockhash);
    // Instructions
    msg.extend_from_slice(&encode_compact_u16(1));
    msg.push(3u8); // program_id index
    // Account indices
    msg.extend_from_slice(&encode_compact_u16(3));
    msg.push(0u8); msg.push(1u8); msg.push(2u8);
    // Data
    msg.extend_from_slice(&encode_compact_u16(ix_data.len() as u16));
    msg.extend_from_slice(ix_data);

    sign_message(oracle, &msg)
}

    program_id: &[u8; 32],
    oracle_pda: &[u8; 32],
    reg_pda:    &[u8; 32],
    blockhash:  &[u8; 32],
    ix_data:    &[u8],
    memo_data:  &[u8],
) -> Vec<u8> {
    let accounts = [oracle, oracle_pda, reg_pda, program_id, &MEMO_PROGRAM_ID];
    let num_accounts = accounts.len() as u8;

    let mut msg = Vec::new();
    msg.extend_from_slice(&[1u8, 0u8, 2u8]);
    msg.extend_from_slice(&encode_compact_u16(num_accounts as u16));
    for acc in &accounts { msg.extend_from_slice(*acc); }
    msg.extend_from_slice(blockhash);
    msg.extend_from_slice(&encode_compact_u16(2));
    // Instruction 1: submit_time
    msg.push(3u8);
    msg.extend_from_slice(&encode_compact_u16(3));
    msg.push(0u8); msg.push(1u8); msg.push(2u8);
    msg.extend_from_slice(&encode_compact_u16(ix_data.len() as u16));
    msg.extend_from_slice(ix_data);
    // Instruction 2: memo
    msg.push(4u8);
    msg.extend_from_slice(&encode_compact_u16(0));
    msg.extend_from_slice(&encode_compact_u16(memo_data.len() as u16));
    msg.extend_from_slice(memo_data);

    sign_message(oracle, &msg)
}



pub fn build_submit_transaction_signed(
    keypair:    &SigningKey,
    program_id: &[u8; 32],
    oracle_pda: &[u8; 32],
    reg_pda:    &[u8; 32],
    blockhash:  &[u8; 32],
    params:     &SubmitParams,
) -> Vec<u8> {
    use ed25519_dalek::Signer;

    let discriminator = anchor_discriminator("submit_time");
    let mut ix_data = discriminator.to_vec();
    ix_data.extend_from_slice(&params.consensus.timestamp_ms.to_le_bytes());
    let spread16 = (params.consensus.spread_ms as i16).to_le_bytes();
    ix_data.extend_from_slice(&spread16);
    ix_data.push(params.consensus.sources_used);
    ix_data.push((params.consensus.confidence * 100.0) as u8);
    ix_data.extend_from_slice(&params.consensus.sources_bitmap.to_le_bytes());

    let ntp_ms   = params.consensus.timestamp_ms;
    let ntp_s    = ntp_ms / 1000;
    let ntp_frac = ntp_ms % 1000;
    let ntp_h    = (ntp_s % 86400) / 3600;
    let ntp_m    = (ntp_s % 3600) / 60;
    let ntp_sec  = ntp_s % 60;
    let chain_str = if let Some(cms) = params.chain_time_ms {
        let cs = cms / 1000;
        format!("{:02}:{:02}:{:02}.0000", (cs%86400)/3600, (cs%3600)/60, cs%60)
    } else { "??:??:??.????".to_string() };
    let best_stratum = params.consensus.sources.iter().map(|r| r.stratum).min().unwrap_or(1);
    let memo_str = format!(
        "strontium:v1:w={}:ntp={:02}:{:02}:{:02}.{:04}:chain={}:c={}:s={}:st={}",
        params.window_id, ntp_h, ntp_m, ntp_sec, ntp_frac*10,
        chain_str, (params.consensus.confidence*100.0) as u8,
        params.consensus.sources_used, best_stratum
    );
    let memo_data = memo_str.as_bytes().to_vec();

    let oracle_bytes: [u8; 32] = keypair.verifying_key().to_bytes();
    let accounts: &[&[u8; 32]] = &[&oracle_bytes, oracle_pda, reg_pda, program_id, &MEMO_PROGRAM_ID];

    let mut msg = Vec::new();
    let num_accounts = if params.memo_enabled { 5u16 } else { 4u16 };
    msg.extend_from_slice(&[1u8, 0u8, if params.memo_enabled { 2u8 } else { 1u8 }]);
    msg.extend_from_slice(&encode_compact_u16(num_accounts));
    let acc_count = if params.memo_enabled { 5 } else { 4 };
    for acc in accounts.iter().take(acc_count) { msg.extend_from_slice(*acc); }
    msg.extend_from_slice(blockhash);

    if params.memo_enabled {
        msg.extend_from_slice(&encode_compact_u16(2));
        // IX 1: submit_time
        msg.push(3u8);
        msg.extend_from_slice(&encode_compact_u16(3));
        msg.push(0); msg.push(1); msg.push(2);
        msg.extend_from_slice(&encode_compact_u16(ix_data.len() as u16));
        msg.extend_from_slice(&ix_data);
        // IX 2: memo
        msg.push(4u8);
        msg.extend_from_slice(&encode_compact_u16(0));
        msg.extend_from_slice(&encode_compact_u16(memo_data.len() as u16));
        msg.extend_from_slice(&memo_data);
    } else {
        msg.extend_from_slice(&encode_compact_u16(1));
        msg.push(3u8);
        msg.extend_from_slice(&encode_compact_u16(3));
        msg.push(0); msg.push(1); msg.push(2);
        msg.extend_from_slice(&encode_compact_u16(ix_data.len() as u16));
        msg.extend_from_slice(&ix_data);
    }

    let sig = keypair.sign(&msg);
    let mut tx = Vec::new();
    tx.extend_from_slice(&encode_compact_u16(1));
    tx.extend_from_slice(sig.to_bytes().as_ref());
    tx.extend_from_slice(&msg);
    tx
}

// ── Registration TX ───────────────────────────────────────────────────────────

pub fn build_register_transaction(
    keypair:    &SigningKey,
    vote_keypair: &SigningKey,
    program_id: &[u8; 32],
    reg_pda:    &[u8; 32],
    blockhash:  &[u8; 32],
) -> Vec<u8> {
    use ed25519_dalek::Signer;

    let system_program = [0u8; 32];
    let discriminator  = anchor_discriminator("register_submitter");
    let ix_data        = discriminator.to_vec();

    let oracle_bytes: [u8; 32] = keypair.verifying_key().to_bytes();
    let vote_bytes:   [u8; 32] = vote_keypair.verifying_key().to_bytes();

    let accounts: &[&[u8; 32]] = &[&oracle_bytes, &vote_bytes, reg_pda, &system_program, program_id];

    let mut msg = Vec::new();
    msg.extend_from_slice(&[2u8, 0u8, 1u8]); // 2 signers
    msg.extend_from_slice(&encode_compact_u16(5));
    for acc in accounts { msg.extend_from_slice(*acc); }
    msg.extend_from_slice(blockhash);
    msg.extend_from_slice(&encode_compact_u16(1));
    msg.push(4u8); // program_id index
    msg.extend_from_slice(&encode_compact_u16(3));
    msg.push(0); msg.push(1); msg.push(2);
    msg.extend_from_slice(&encode_compact_u16(ix_data.len() as u16));
    msg.extend_from_slice(&ix_data);

    let sig1 = keypair.sign(&msg);
    let sig2 = vote_keypair.sign(&msg);

    let mut tx = Vec::new();
    tx.extend_from_slice(&encode_compact_u16(2));
    tx.extend_from_slice(sig1.to_bytes().as_ref());
    tx.extend_from_slice(sig2.to_bytes().as_ref());
    tx.extend_from_slice(&msg);
    tx
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn lamports_to_xnt(lamports: u64) -> f64 { lamports as f64 / 1_000_000_000.0 }

pub fn estimate_days_remaining(balance_xnt: f64, interval_s: u64) -> f64 {
    let tx_per_day = 86400.0 / interval_s as f64;
    let cost_per_day = tx_per_day * 0.002; // ~0.002 XNT per TX (submit + memo)
    if cost_per_day <= 0.0 { return f64::INFINITY; }
    balance_xnt / cost_per_day
}

pub fn parse_blockhash(resp: &str) -> Result<[u8; 32], String> {
    let prefix = "\"blockhash\":\"";
    let start  = resp.find(prefix).ok_or("No blockhash in response")?;
    let rest   = &resp[start + prefix.len()..];
    let end    = rest.find('"').ok_or("Unterminated blockhash")?;
    let bytes  = bs58::decode(&rest[..end]).into_vec()
        .map_err(|e| format!("Blockhash decode: {}", e))?;
    bytes.try_into().map_err(|_| "Blockhash wrong length".to_string())
}

fn parse_balance(resp: &str) -> Result<u64, String> {
    let prefix = "\"value\":";
    let start  = resp.find(prefix).ok_or("No value in response")?;
    let rest   = resp[start + prefix.len()..].trim_start();
    let end    = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse::<u64>().map_err(|e| format!("Balance parse: {}", e))
}

fn parse_signature(resp: &str) -> Result<String, String> {
    if resp.contains("\"error\"") {
        let msg = resp.find("\"message\":\"")
            .map(|i| {
                let s = &resp[i + 11..];
                s[..s.find('"').unwrap_or(s.len())].to_string()
            })
            .unwrap_or_else(|| resp.to_string());
        return Err(format!("RPC error: {}", msg));
    }
    let prefix = "\"result\":\"";
    let start  = resp.find(prefix).ok_or("No result in response")?;
    let rest   = &resp[start + prefix.len()..];
    let end    = rest.find('"').ok_or("Unterminated result")?;
    Ok(rest[..end].to_string())
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub fn base64_encode(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let c  = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(c >> 18) & 63] as char);
        out.push(A[(c >> 12) & 63] as char);
        out.push(if chunk.len() > 1 { A[(c >> 6) & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[c & 63] as char } else { '=' });
    }
    out
}
