use std::time::{Duration, Instant};
use sha2::{Digest, Sha256};
use ed25519_dalek::{Keypair, Signer};
use crate::consensus::ConsensusResult;

#[allow(dead_code)]
pub const PROGRAM_ID: &str = "2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe";
#[allow(dead_code)]
pub const ORACLE_PDA: &str = "EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn";
pub const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

// ─── Circuit Breaker ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct CircuitBreaker {
    failures:     u32,
    last_failure: Option<Instant>,
    open_until:   Option<Instant>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self { failures: 0, last_failure: None, open_until: None }
    }

    fn is_open(&self) -> bool {
        if let Some(until) = self.open_until {
            Instant::now() < until
        } else {
            false
        }
    }

    fn record_failure(&mut self) {
        self.failures += 1;
        self.last_failure = Some(Instant::now());
        if self.failures >= 3 {
            // Circuit open for 5 minutes
            self.open_until = Some(Instant::now() + Duration::from_secs(300));
            eprintln!("[circuit] RPC circuit breaker opened for 5 minutes");
        }
    }

    fn record_success(&mut self) {
        self.failures = 0;
        self.open_until = None;
        self.last_failure = None;
    }
}

// ─── RPC Client ───────────────────────────────────────────────────────────────

pub struct RpcClient {
    urls:            Vec<String>,
    current_idx:     usize,
    circuit_breakers: Vec<CircuitBreaker>,
}

impl RpcClient {
    pub fn new(urls: Vec<String>) -> Self {
        let n = urls.len();
        Self {
            urls,
            current_idx: 0,
            circuit_breakers: (0..n).map(|_| CircuitBreaker::new()).collect(),
        }
    }

    /// Get current active RPC URL (skips broken endpoints)
    pub fn active_url(&self) -> Option<&str> {
        // Try current, then fallbacks
        for offset in 0..self.urls.len() {
            let idx = (self.current_idx + offset) % self.urls.len();
            if !self.circuit_breakers[idx].is_open() {
                return Some(&self.urls[idx]);
            }
        }
        None
    }

    /// Check if an RPC endpoint is healthy (responds + slot is fresh)
    #[allow(dead_code)]
    pub fn check_health(&self, url: &str) -> bool {
        let payload = r#"{"jsonrpc":"2.0","id":1,"method":"getHealth"}"#;
        match self.post_json(url, payload, 3000) {
            Ok(resp) => resp.contains("\"ok\"") || resp.contains("\"result\""),
            Err(_) => false,
        }
    }

    /// Get latest blockhash from RPC with retry and fallback
    pub fn get_recent_blockhash(&mut self) -> Result<[u8; 32], String> {
        let payload = r#"{"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[{"commitment":"confirmed"}]}"#;
        self.rpc_call_with_retry(payload, |resp| {
            parse_blockhash(&resp)
        })
    }

    /// Get account balance in lamports
    pub fn get_balance(&mut self, pubkey: &str) -> Result<u64, String> {
        let payload = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"getBalance","params":["{}", {{"commitment":"confirmed"}}]}}"#,
            pubkey
        );
        self.rpc_call_with_retry(&payload, |resp| {
            parse_balance(&resp)
        })
    }

    /// Send a signed transaction
    pub fn send_transaction(&mut self, tx_base64: &str) -> Result<String, String> {
        let payload = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":["{}",{{"encoding":"base64","skipPreflight":false}}]}}"#,
            tx_base64
        );
        self.rpc_call_with_retry(&payload, |resp| {
            parse_signature(&resp)
        })
    }

    /// Generic RPC call with retry (3x exponential backoff) and endpoint failover
    fn rpc_call_with_retry<T, F>(&mut self, payload: &str, parser: F) -> Result<T, String>
    where
        F: Fn(String) -> Result<T, String>,
    {
        let mut last_err = String::from("No RPC endpoints available");

        for _attempt in 0..3 {
            // Find healthy endpoint
            let url = match self.find_healthy_endpoint() {
                Some(u) => u,
                None => break,
            };

            match self.post_json(&url, payload, 10_000) {
                Ok(resp) => {
                    match parser(resp) {
                        Ok(val) => {
                            self.circuit_breakers[self.current_idx].record_success();
                            return Ok(val);
                        }
                        Err(e) => {
                            last_err = e;
                            // Don't mark as circuit failure for parse errors
                        }
                    }
                }
                Err(e) => {
                    last_err = e;
                    self.circuit_breakers[self.current_idx].record_failure();
                    // Try next endpoint
                    self.failover();
                }
            }

            // Exponential backoff: 2s, 4s, 8s
            let delay = 2u64 << _attempt;
            std::thread::sleep(Duration::from_secs(delay.min(8)));
        }

        Err(last_err)
    }

    fn find_healthy_endpoint(&mut self) -> Option<String> {
        for offset in 0..self.urls.len() {
            let idx = (self.current_idx + offset) % self.urls.len();
            if !self.circuit_breakers[idx].is_open() {
                self.current_idx = idx;
                return Some(self.urls[idx].clone());
            }
        }
        None
    }

    fn failover(&mut self) {
        self.current_idx = (self.current_idx + 1) % self.urls.len();
        eprintln!("[rpc] Failing over to: {}", self.urls[self.current_idx]);
    }

    fn post_json(&self, url: &str, payload: &str, timeout_ms: u64) -> Result<String, String> {
        ureq::post(url)
            .set("Content-Type", "application/json")
            .set("User-Agent", "X1-Strontium/1.0 (https://github.com/PioWin-clo/strontium)")
            .timeout(Duration::from_millis(timeout_ms))
            .send_string(payload)
            .map_err(|e| format!("HTTP error: {}", e))?
            .into_string()
            .map_err(|e| format!("Response read error: {}", e))
    }
}

// ─── Transaction Builder ──────────────────────────────────────────────────────

/// Build and sign a submit_time transaction with embedded Memo
pub fn build_submit_transaction(
    keypair:       &Keypair,
    program_id:    &[u8; 32],
    oracle_pda:    &[u8; 32],
    reg_pda:       &[u8; 32],
    blockhash:     &[u8; 32],
    consensus:     &ConsensusResult,
    window_id:     u64,
) -> Vec<u8> {
    let oracle_pubkey: [u8; 32] = keypair.public.to_bytes();

    // Build submit_time instruction data
    let discriminator = compute_discriminator("submit_time");
    let mut ix_data = discriminator.to_vec();
    ix_data.extend_from_slice(&consensus.timestamp_ms.to_le_bytes());  // i64
    ix_data.extend_from_slice(&(consensus.spread_ms as i16).to_le_bytes()); // i16
    ix_data.push(consensus.sources_used);            // u8
    ix_data.push((consensus.confidence * 100.0) as u8);               // u8 confidence_pct
    ix_data.push(consensus.sources_bitmap);                           // u8 sources_bitmap

    // Build Memo instruction data
    let memo_str = format!(
        "strontium:v1:w={}:t={}:c={}:s={}",
        window_id,
        consensus.timestamp_ms,
        (consensus.confidence * 100.0) as u8,
        consensus.sources_used
    );
    let memo_data = memo_str.as_bytes().to_vec();

    // Build message with 2 instructions: submit_time + memo
    let msg = build_message_two_instructions(
        &oracle_pubkey,
        program_id,
        oracle_pda,
        reg_pda,
        blockhash,
        &ix_data,
        &memo_data,
    );

    // Sign
    let signature = keypair.sign(&msg);
    let sig_bytes = signature.to_bytes();

    let mut tx = Vec::new();
    tx.push(1u8); // 1 signature
    tx.extend_from_slice(&sig_bytes);
    tx.extend_from_slice(&msg);
    tx
}

/// Build Solana message with submit_time + memo instructions
fn build_message_two_instructions(
    oracle:       &[u8; 32],
    program_id:   &[u8; 32],
    oracle_pda:   &[u8; 32],
    reg_pda:      &[u8; 32],
    blockhash:    &[u8; 32],
    ix_data:      &[u8],
    memo_data:    &[u8],
) -> Vec<u8> {
    // Decode memo program ID
    let memo_prog = bs58::decode(MEMO_PROGRAM).into_vec()
        .unwrap_or_else(|_| vec![0u8; 32]);
    let memo_prog_arr: [u8; 32] = memo_prog.try_into().unwrap_or([0u8; 32]);

    let mut msg = Vec::new();

    // Header: [num_signers=1, num_readonly_signed=0, num_readonly_unsigned=3]
    // (oracle_pda, reg_pda, program_id, memo_program = 4 accounts total after oracle)
    msg.extend_from_slice(&[1u8, 0u8, 3u8]);

    // Account list (5): oracle, oracle_pda, reg_pda, program_id, memo_program
    encode_compact_u16(&mut msg, 5);
    msg.extend_from_slice(oracle);        // 0: signer + writable
    msg.extend_from_slice(oracle_pda);    // 1: writable non-signer
    msg.extend_from_slice(reg_pda);       // 2: readonly non-signer
    msg.extend_from_slice(program_id);    // 3: program (readonly)
    msg.extend_from_slice(&memo_prog_arr);// 4: memo program (readonly)

    // Blockhash
    msg.extend_from_slice(blockhash);

    // 2 instructions
    encode_compact_u16(&mut msg, 2);

    // Instruction 1: submit_time
    msg.push(3u8); // program_id_index
    encode_compact_u16(&mut msg, 3);
    msg.push(1u8); // oracle_pda
    msg.push(0u8); // oracle (signer)
    msg.push(2u8); // registration
    encode_compact_u16(&mut msg, ix_data.len() as u16);
    msg.extend_from_slice(ix_data);

    // Instruction 2: memo (Memo Program)
    msg.push(4u8); // memo_program_index
    encode_compact_u16(&mut msg, 0); // no accounts needed for memo
    encode_compact_u16(&mut msg, memo_data.len() as u16);
    msg.extend_from_slice(memo_data);

    msg
}

// ─── Registration Transaction ─────────────────────────────────────────────────

pub fn build_register_transaction(
    oracle_keypair: &Keypair,
    vote_keypair:   &Keypair,
    program_id:     &[u8; 32],
    blockhash:      &[u8; 32],
) -> Vec<u8> {
    let oracle_pubkey:    [u8; 32] = oracle_keypair.public.to_bytes();
    let vote_pubkey:      [u8; 32] = vote_keypair.public.to_bytes();
    let reg_pda = derive_registration_pda(&oracle_pubkey, program_id);
    let system_prog = [0u8; 32]; // system program

    let discriminator = compute_discriminator("register_submitter");
    let ix_data: Vec<u8> = discriminator.to_vec();

    let msg = build_register_message(
        &oracle_pubkey,
        &vote_pubkey,
        program_id,
        &reg_pda,
        &system_prog,
        blockhash,
        &ix_data,
    );

    // Sign with both keypairs
    let sig_oracle = oracle_keypair.sign(&msg);
    let sig_vote   = vote_keypair.sign(&msg);

    let mut tx = Vec::new();
    tx.push(2u8); // 2 signatures
    tx.extend_from_slice(&sig_oracle.to_bytes());
    tx.extend_from_slice(&sig_vote.to_bytes());
    tx.extend_from_slice(&msg);
    tx
}

fn build_register_message(
    oracle:     &[u8; 32],
    vote:       &[u8; 32],
    program_id: &[u8; 32],
    reg_pda:    &[u8; 32],
    system:     &[u8; 32],
    blockhash:  &[u8; 32],
    ix_data:    &[u8],
) -> Vec<u8> {
    let mut msg = Vec::new();

    // Header: 2 signers, 0 readonly signed, 2 readonly unsigned (program + system)
    msg.extend_from_slice(&[2u8, 0u8, 2u8]);

    // Account list (5): oracle, vote, reg_pda, program_id, system
    encode_compact_u16(&mut msg, 5);
    msg.extend_from_slice(oracle);     // 0: signer + writable (oracle + payer)
    msg.extend_from_slice(vote);       // 1: signer readonly
    msg.extend_from_slice(reg_pda);    // 2: writable (being created)
    msg.extend_from_slice(program_id); // 3: readonly
    msg.extend_from_slice(system);     // 4: readonly (system program)

    msg.extend_from_slice(blockhash);

    encode_compact_u16(&mut msg, 1);
    msg.push(3u8); // program_id_index

    // Instruction accounts: oracle_keypair(0), vote(1), reg_pda(2), system(4)
    encode_compact_u16(&mut msg, 4);
    msg.push(0u8); // oracle_keypair (payer + oracle)
    msg.push(1u8); // vote_account
    msg.push(2u8); // registration PDA
    msg.push(4u8); // system_program

    encode_compact_u16(&mut msg, ix_data.len() as u16);
    msg.extend_from_slice(ix_data);

    msg
}

// ─── PDA Derivation ───────────────────────────────────────────────────────────

pub fn derive_registration_pda(oracle_pubkey: &[u8; 32], program_id: &[u8; 32]) -> [u8; 32] {
    for bump in (0u8..=255).rev() {
        let mut h = Sha256::new();
        h.update(b"reg");
        h.update(oracle_pubkey);
        h.update([bump]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let hash = h.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash[..32]);
        if is_off_curve(&arr) {
            return arr;
        }
    }
    [0u8; 32]
}

fn is_off_curve(bytes: &[u8; 32]) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*bytes)
        .decompress()
        .is_none()
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn compute_discriminator(name: &str) -> [u8; 8] {
    let preimage = format!("global:{}", name);
    let mut h = Sha256::new();
    h.update(preimage.as_bytes());
    let hash = h.finalize();
    let mut out = [0u8; 8];
    out.copy_from_slice(&hash[..8]);
    out
}

fn encode_compact_u16(buf: &mut Vec<u8>, val: u16) {
    let mut v = val;
    loop {
        let mut b = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 { b |= 0x80; }
        buf.push(b);
        if v == 0 { break; }
    }
}

/// Parse blockhash from RPC response
pub fn parse_blockhash(resp: &str) -> Result<[u8; 32], String> {
    let prefix = "\"blockhash\":\"";
    let start  = resp.find(prefix).ok_or("No blockhash in response")?;
    let rest   = &resp[start + prefix.len()..];
    let end    = rest.find('"').ok_or("Unterminated blockhash")?;
    let bh_str = &rest[..end];
    let bytes  = bs58::decode(bh_str).into_vec()
        .map_err(|e| format!("Blockhash decode: {}", e))?;
    bytes.try_into().map_err(|_| "Blockhash wrong length".to_string())
}

fn parse_balance(resp: &str) -> Result<u64, String> {
    let prefix = "\"value\":";
    let start  = resp.find(prefix).ok_or("No value in response")?;
    let rest   = &resp[start + prefix.len()..].trim_start_matches(' ');
    let end    = rest.find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse::<u64>().map_err(|e| format!("Balance parse: {}", e))
}

fn parse_signature(resp: &str) -> Result<String, String> {
    if resp.contains("\"error\"") {
        let msg = resp.find("\"message\":\"")
            .map(|i| {
                let s = &resp[i + 11..];
                let e = s.find('"').unwrap_or(s.len());
                s[..e].to_string()
            })
            .unwrap_or_else(|| resp.to_string());
        return Err(format!("RPC error: {}", msg));
    }
    let prefix = "\"result\":\"";
    let start  = resp.find(prefix).ok_or("No result in response")?;
    let rest   = &resp[start + prefix.len()..];
    let end    = rest.find('"').ok_or("Unterminated signature")?;
    Ok(rest[..end].to_string())
}

/// Lamports to XNT
pub fn lamports_to_xnt(lamports: u64) -> f64 {
    lamports as f64 / 1_000_000_000.0
}

/// Estimate days remaining based on current balance and daily tx cost
pub fn estimate_days_remaining(balance_xnt: f64, interval_s: u64) -> f64 {
    let tx_per_day    = 86400.0 / interval_s as f64;
    let fee_per_tx    = 0.002; // XNT
    let cost_per_day  = tx_per_day * fee_per_tx;
    if cost_per_day == 0.0 { return f64::INFINITY; }
    balance_xnt / cost_per_day
}
