use std::io::{self, Write};
use std::path::Path;
use ed25519_dalek::Keypair;
use crate::submitter::{
    RpcClient, build_register_transaction, derive_registration_pda,
    lamports_to_xnt,
};
use crate::config::StrontiumConfig;

pub const MIN_STAKE_XNT: f64 = 100.0;
pub const MIN_BALANCE_FOR_REGISTER: f64 = 0.05; // XNT needed for registration TX

/// Full registration flow
pub fn run_register(config: &mut StrontiumConfig) -> Result<(), String> {
    println!("╔═══════════════════════════════════════╗");
    println!("║   X1 Strontium — Register Validator  ║");
    println!("╚═══════════════════════════════════════╝");

    // Load oracle keypair
    let oracle_kp = load_keypair(&config.keypair_path)
        .map_err(|e| format!("Oracle keypair error: {}", e))?;
    let oracle_pubkey = bs58::encode(oracle_kp.public.to_bytes()).into_string();

    println!("  Oracle keypair : {}", oracle_pubkey);

    // Resolve vote keypair path
    let vote_path = resolve_vote_keypair_path(config)?;
    let vote_kp   = load_keypair(&vote_path)
        .map_err(|e| format!("Vote keypair error: {}", e))?;
    let vote_pubkey = bs58::encode(vote_kp.public.to_bytes()).into_string();

    println!("  Vote keypair   : {}", vote_pubkey);
    println!("  Program ID     : {}", config.program_id);

    // Initialize RPC client
    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    // Check oracle balance
    println!("\n⏳ Checking oracle keypair balance...");
    let balance_lamps = rpc.get_balance(&oracle_pubkey)
        .map_err(|e| format!("Balance check failed: {}", e))?;
    let balance_xnt = lamports_to_xnt(balance_lamps);
    println!("  Balance: {:.3} XNT", balance_xnt);

    if balance_xnt < MIN_BALANCE_FOR_REGISTER {
        return Err(format!(
            "Insufficient balance: {:.3} XNT\n  Minimum required: {} XNT\n  Fund your oracle keypair:\n  solana transfer {} 1 --url {} --keypair <YOUR_WALLET> --allow-unfunded-recipient",
            balance_xnt, MIN_BALANCE_FOR_REGISTER,
            oracle_pubkey,
            config.rpc_urls.get(1).map(|s| s.as_str()).unwrap_or("https://rpc.mainnet.x1.xyz")
        ));
    }

    // Validate validator via api.x1.xyz
    println!("\n⏳ Validating validator status via api.x1.xyz...");
    match validate_validator_api(&vote_pubkey) {
        Ok(()) => println!("  ✓ Validator is active with sufficient stake"),
        Err(e) => {
            eprintln!("  ⚠ Validator check warning: {}", e);
            eprintln!("  Proceeding with registration (api.x1.xyz may be unavailable)");
        }
    }

    // Derive registration PDA
    let program_id_bytes = bs58::decode(&config.program_id).into_vec()
        .map_err(|_| "Invalid program ID".to_string())?;
    let program_id_arr: [u8; 32] = program_id_bytes.try_into()
        .map_err(|_| "Program ID wrong length".to_string())?;

    let oracle_bytes: [u8; 32] = oracle_kp.public.to_bytes();
    let reg_pda_bytes = derive_registration_pda(&oracle_bytes, &program_id_arr);
    let reg_pda = bs58::encode(reg_pda_bytes).into_string();

    // Compute bump (just for display)
    let bump = find_bump(&oracle_bytes, &program_id_arr);

    println!("\n  Oracle keypair : {}", oracle_pubkey);
    println!("  Vote account   : {}", vote_pubkey);
    println!("  Registration   : {} (bump={})", reg_pda, bump);

    // Get blockhash
    println!("\n⏳ Sending registration transaction...");
    println!("   (requires both oracle + vote signatures)");

    let rpc_url = rpc.active_url()
        .ok_or("No healthy RPC endpoint")?
        .to_string();

    let blockhash_bytes = rpc.get_recent_blockhash()
        .map_err(|e| format!("Blockhash error: {}", e))?;

    // Build and sign transaction
    let tx = build_register_transaction(
        &oracle_kp,
        &vote_kp,
        &program_id_arr,
        &blockhash_bytes,
    );

    // Encode to base64
    use std::io::Read;
    let tx_b64 = base64_encode(&tx);

    // Send
    let sig = rpc.send_transaction(&tx_b64)
        .map_err(|e| format!("Registration failed: {}", e))?;

    println!("✓ Registration successful!");
    println!("  TX: {}", shorten(&sig, 8));
    println!("  Explorer: https://explorer.mainnet.x1.xyz/tx/{}", sig);

    // Save vote keypair path to config
    config.vote_keypair_path = Some(vote_path);
    config.save()
        .unwrap_or_else(|e| eprintln!("[warn] Could not save config: {}", e));

    println!("\nYour validator is now registered. Start the daemon:");
    println!("  x1sr start");
    println!("  (or: strontium start --keypair {})", config.keypair_path);

    Ok(())
}

/// Validate validator via api.x1.xyz — checks isActive, skipRate, self-stake
fn validate_validator_api(vote_pubkey: &str) -> Result<(), String> {
    let url = format!("https://api.x1.xyz/v1/validators/{}", vote_pubkey);

    let resp = ureq::get(&url)
        .set("User-Agent", "X1-Strontium/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| format!("API request failed: {}", e))?
        .into_string()
        .map_err(|e| format!("API response error: {}", e))?;

    // Check isActive
    if resp.contains("\"isActive\":false") || resp.contains("\"status\":\"delinquent\"") {
        return Err(format!(
            "Validator is not active. Ensure your validator is running and healthy.\n  Vote account: {}",
            vote_pubkey
        ));
    }

    // Check skipRate < 10%
    if let Some(skip_rate) = parse_float_field(&resp, "skipRate") {
        if skip_rate > 0.10 {
            return Err(format!(
                "Skip rate too high: {:.1}% (maximum: 10%)\n  Improve validator performance before registering.",
                skip_rate * 100.0
            ));
        }
    }

    // Check verifiedSelfStake >= 100 XNT
    if let Some(stake) = parse_float_field(&resp, "verifiedSelfStakeCurrentEpoch") {
        if stake < MIN_STAKE_XNT {
            return Err(format!(
                "Insufficient self-stake: {:.0} XNT (minimum: {} XNT)\n  Increase your self-stake and try again.",
                stake, MIN_STAKE_XNT
            ));
        }
    }

    Ok(())
}

/// Resolve vote keypair path — use config, auto-detect, or prompt user
fn resolve_vote_keypair_path(config: &StrontiumConfig) -> Result<String, String> {
    // 1. Already in config
    if let Some(path) = &config.vote_keypair_path {
        if Path::new(path).exists() {
            return Ok(path.clone());
        }
    }

    // 2. Default location
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let default_path = Path::new(&home).join(".config").join("solana").join("vote.json");
    if default_path.exists() {
        println!("  Vote keypair   : {} (auto-detected)", default_path.display());
        return Ok(default_path.to_string_lossy().to_string());
    }

    // 3. Ask user
    println!("  Vote keypair not found at default location (~/.config/solana/vote.json)");
    print!("  Enter path to your vote keypair: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input)
        .map_err(|e| format!("Input error: {}", e))?;
    let path = input.trim().to_string();

    if path.is_empty() {
        return Err("Vote keypair path is required for registration".to_string());
    }

    if !Path::new(&path).exists() {
        return Err(format!("File not found: {}", path));
    }

    Ok(path)
}

/// Load an ed25519 keypair from a JSON file (Solana format: [u8; 64])
pub fn load_keypair(path: &str) -> Result<Keypair, String> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read {}: {}", path, e))?;

    // Parse JSON array of bytes
    let bytes: Vec<u8> = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid keypair JSON in {}: {}", path, e))?;

    if bytes.len() != 64 {
        return Err(format!("Keypair must be 64 bytes, got {} in {}", bytes.len(), path));
    }

    Keypair::from_bytes(&bytes)
        .map_err(|e| format!("Invalid keypair in {}: {}", path, e))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn parse_float_field(json: &str, field: &str) -> Option<f64> {
    let key = format!("\"{}\":", field);
    let start = json.find(&key)? + key.len();
    let rest  = json[start..].trim_start();
    let end   = rest.find(|c: char| c == ',' || c == '}' || c == ']')
        .unwrap_or(rest.len());
    rest[..end].trim().parse::<f64>().ok()
}

fn find_bump(oracle_pubkey: &[u8; 32], program_id: &[u8; 32]) -> u8 {
    use sha2::{Digest, Sha256};
    for bump in (0u8..=255).rev() {
        let mut h = Sha256::new();
        h.update(b"reg");
        h.update(oracle_pubkey);
        h.update(&[bump]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let hash = h.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash[..32]);
        if curve25519_dalek::edwards::CompressedEdwardsY(arr).decompress().is_none() {
            return bump;
        }
    }
    0
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(combined >> 18) & 63] as char);
        out.push(ALPHABET[(combined >> 12) & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(combined >> 6) & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[combined & 63] as char } else { '=' });
    }
    out
}

fn shorten(s: &str, n: usize) -> String {
    if s.len() <= n * 2 + 3 { return s.to_string(); }
    format!("{}...{}", &s[..n], &s[s.len()-n..])
}
