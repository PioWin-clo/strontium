use std::fs;
use std::path::Path;
use ed25519_dalek::SigningKey;

use crate::config::StrontiumConfig;
use crate::submitter::{RpcClient, build_register_transaction, lamports_to_xnt,
                        derive_registration_pda, base64_encode};

pub fn load_keypair(path: &str) -> Result<SigningKey, String> {
    let data = fs::read_to_string(path)
        .map_err(|e| format!("Cannot read keypair {}: {}", path, e))?;
    let bytes: Vec<u8> = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid keypair JSON: {}", e))?;
    if bytes.len() != 64 {
        return Err(format!("Keypair must be 64 bytes, got {}", bytes.len()));
    }
    let secret: [u8; 32] = bytes[..32].try_into()
        .map_err(|_| "Invalid secret key".to_string())?;
    Ok(SigningKey::from_bytes(&secret))
}

pub fn run_register(config: &mut StrontiumConfig) -> Result<(), String> {
    println!("X1 Strontium — registering oracle");
    println!("{}", "━".repeat(50));

    let keypair = load_keypair(&config.keypair_path)
        .map_err(|e| format!("Oracle keypair error: {}", e))?;
    let oracle_pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();

    let vote_path = config.vote_keypair_path.as_ref()
        .ok_or("vote_keypair not configured — run: x1sr config set vote_keypair <path>")?
        .clone();
    let vote_keypair = load_keypair(&vote_path)
        .map_err(|e| format!("Vote keypair error: {}", e))?;
    let vote_pubkey = bs58::encode(vote_keypair.verifying_key().to_bytes()).into_string();

    println!("  Oracle keypair : {}", oracle_pubkey);
    println!("  Vote account   : {}", vote_pubkey);

    let program_id_bytes = bs58::decode(&config.program_id).into_vec()
        .map_err(|_| "Invalid program ID")?;
    let program_id: [u8; 32] = program_id_bytes.try_into()
        .map_err(|_| "Program ID wrong length")?;

    let oracle_bytes: [u8; 32] = keypair.verifying_key().to_bytes();
    let reg_pda = derive_registration_pda(&oracle_bytes, &program_id);
    let reg_pda_str = bs58::encode(reg_pda).into_string();
    println!("  Registration   : {}", reg_pda_str);

    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    // Check balance
    let balance = rpc.get_balance(&oracle_pubkey)
        .map(lamports_to_xnt)
        .unwrap_or(0.0);
    println!("  Balance        : {:.3} XNT", balance);
    if balance < 0.05 {
        return Err(format!("Insufficient balance ({:.3} XNT) — need at least 0.05 XNT", balance));
    }

    // Get blockhash
    let blockhash = rpc.get_recent_blockhash()
        .map_err(|e| format!("Cannot get blockhash: {}", e))?;

    // Build and send registration TX
    let tx = build_register_transaction(
        &keypair, &vote_keypair, &program_id, &reg_pda, &blockhash,
    );
    let tx_b64 = base64_encode(&tx);

    print!("  Sending registration TX... ");
    match rpc.send_transaction(&tx_b64) {
        Ok(sig) => {
            println!("✅");
            println!("  Signature      : {}", sig);
            println!("  Registration valid for 90 days.");
            println!();
            println!("✅ Registration complete! Start the daemon:");
            println!("   x1sr start");
            Ok(())
        }
        Err(e) => Err(format!("Registration failed: {}", e)),
    }
}
