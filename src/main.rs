mod config;
mod status;
mod ntp_client;
mod consensus;
mod submitter;
mod register;

use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config::{StrontiumConfig, ORACLE_PDA};
use status::{DaemonStatus, SilentReason};
use ntp_client::{discover_sources, query_sources_parallel, has_gps_pps};
use consensus::{run_consensus_cycle, rotation_my_turn};
use submitter::{RpcClient, build_submit_transaction, derive_registration_pda,
                lamports_to_xnt, estimate_days_remaining};
use register::{run_register, load_keypair};

// ─── Constants ────────────────────────────────────────────────────────────────

const MIN_BALANCE_WARN:   f64 = 1.0;  // XNT — show warning
const MIN_BALANCE_STOP:   f64 = 0.05; // XNT — stop daemon
const REDISCOVER_SECS:    u64 = 3600; // Re-discover NTP sources every hour
const READINESS_MAX_TRIES: u32 = 20;  // Max readiness check attempts (20 min)

// ─── Entry Point ─────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || args[1] == "--help" || args[1] == "help" || args[1] == "-h" {
        print_help();
        return;
    }

    let subcmd = args[1].as_str();

    match subcmd {
        "start" => {
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[2..]);
            run_daemon(config);
        }
        "stop" => cmd_stop(),
        "status" => cmd_status(),
        "sources" => cmd_sources(),
        "history" => cmd_history(&args[2..]),
        "config" => cmd_config(&args[2..]),
        "register" => {
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[2..]);
            if let Err(e) = run_register(&mut config) {
                eprintln!("✗ {}", e);
                std::process::exit(1);
            }
        }
        "deregister" => cmd_deregister(),
        "balance" => cmd_balance(),
        "archive" => cmd_archive(&args[2..]),
        "install" => cmd_install(),
        "uninstall" => cmd_uninstall(),
        "--keypair" | "--dry-run" | "--interval" | "--rpc" => {
            // Legacy mode: strontium --keypair ...
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[1..]);
            run_daemon(config);
        }
        _ => {
            eprintln!("Unknown command: {}", subcmd);
            eprintln!("Run 'strontium help' for usage.");
            std::process::exit(1);
        }
    }
}

// ─── Main Daemon Loop ─────────────────────────────────────────────────────────

fn run_daemon(config: StrontiumConfig) {
    println!("╔═══════════════════════════════════════╗");
    println!("║   X1 Strontium — Time Oracle Daemon  ║");
    println!("╚═══════════════════════════════════════╝");

    let keypair = match load_keypair(&config.keypair_path) {
        Ok(kp) => kp,
        Err(e) => { eprintln!("✗ Keypair error: {}", e); std::process::exit(1); }
    };

    let oracle_pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();
    let oracle_bytes: [u8; 32] = keypair.verifying_key().to_bytes();

    let program_id_bytes = bs58::decode(&config.program_id).into_vec()
        .expect("Invalid program ID");
    let program_id_arr: [u8; 32] = program_id_bytes.try_into()
        .expect("Program ID wrong length");
    let oracle_pda_bytes = bs58::decode(&config.oracle_pda).into_vec()
        .expect("Invalid oracle PDA");
    let oracle_pda_arr: [u8; 32] = oracle_pda_bytes.try_into()
        .expect("Oracle PDA wrong length");
    let reg_pda_bytes = derive_registration_pda(&oracle_bytes, &program_id_arr);

    println!("  Keypair   : {}", oracle_pubkey);
    println!("  Program   : {}", config.program_id);
    println!("  PDA       : {}", config.oracle_pda);
    println!("  Interval  : {}s", config.interval_s);
    println!("  Mode      : {}", if config.dry_run { "DRY-RUN (no tx sent)" } else { "live" });

    if has_gps_pps() {
        println!("  GPS/PPS   : detected (/dev/pps0) — using as tier-0 source");
    }

    // Initialize RPC client
    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    // Readiness check — wait until validator is active
    if !config.dry_run {
        readiness_check(&config, &oracle_pubkey, &mut rpc);
    }

    // Initial balance check
    let balance_xnt = check_balance(&oracle_pubkey, &mut rpc);
    if balance_xnt < MIN_BALANCE_STOP {
        eprintln!("✗ Balance too low: {:.3} XNT", balance_xnt);
        eprintln!("  Fund your oracle keypair:");
        eprintln!("  solana transfer {} 1 --url {} --keypair <YOUR_WALLET> --allow-unfunded-recipient",
            oracle_pubkey,
            config.rpc_urls.get(1).map(|s| s.as_str()).unwrap_or("https://rpc.mainnet.x1.xyz")
        );
        std::process::exit(1);
    }

    // Discover NTP sources
    println!("[{}] 🔍 Discovering NTP servers...", now_str());
    let mut ntp_cache = discover_sources(5);
    let mut last_rediscover = unix_secs();
    let mut cycle_num = 0u64;

    print_ntp_sources(&ntp_cache);

    // Main loop
    loop {
        cycle_num += 1;
        let balance_xnt = check_balance(&oracle_pubkey, &mut rpc);
        let days_left = estimate_days_remaining(balance_xnt, config.interval_s);

        // Balance warnings
        if balance_xnt < MIN_BALANCE_STOP {
            eprintln!("✗ Balance critically low: {:.3} XNT — stopping daemon", balance_xnt);
            update_status_silent(SilentReason::InsufficientBalance, balance_xnt, days_left, &config, &oracle_pubkey);
            std::process::exit(1);
        }
        if balance_xnt < MIN_BALANCE_WARN {
            eprintln!("[warn] Balance low: {:.3} XNT (~{:.0} days remaining)", balance_xnt, days_left);
        }

        // Re-discover NTP sources periodically
        if unix_secs() - last_rediscover > REDISCOVER_SECS {
            println!("[{}] 🔍 Re-discovering NTP servers...", now_str());
            ntp_cache = discover_sources(5);
            print_ntp_sources(&ntp_cache);
            last_rediscover = unix_secs();
        }

        // Query NTP sources in parallel
        let results = query_sources_parallel(&ntp_cache);
        println!("[{}] Cycle #{}: {} measurements from {} servers",
            now_str(), cycle_num, results.len(), ntp_cache.len());

        for r in &results {
            println!("  {:<30} rtt={:4}ms  offset={:+6}ms  stratum={}",
                r.host, r.rtt_ms, r.offset_ms, r.stratum);
        }

        // Compute consensus
        let consensus = match run_consensus_cycle(&results) {
            Some(c) => c,
            None => {
                println!("[{}] ✗ No consensus (spread too high or insufficient sources)", now_str());
                update_status_silent(
                    if results.len() < 2 { SilentReason::NoValidSources } else { SilentReason::SpreadTooHigh },
                    balance_xnt, days_left, &config, &oracle_pubkey
                );
                thread::sleep(Duration::from_secs(config.interval_s));
                continue;
            }
        };

        println!("[{}] ✓ Consensus: {} UTC | spread={}ms | sources={} | conf={:.2}{}",
            now_str(),
            format_ts(consensus.timestamp_ms),
            consensus.spread_ms,
            consensus.sources_used,
            consensus.confidence,
            if consensus.is_gps { " [GPS]" } else { "" }
        );

        // Dry run — skip submit
        if config.dry_run {
            println!("[{}] ⚠ [DRY-RUN] tx skipped", now_str());
            update_status_dry_run(&consensus, balance_xnt, days_left, &config, &oracle_pubkey);
            thread::sleep(Duration::from_secs(config.interval_s));
            continue;
        }

        // Rotation check — is it my turn?
        // For now n_validators=1 until we implement on-chain committee reading
        let n_validators = 1usize; // TODO: read from on-chain in future version
        let (is_my_turn, window_id, _) = rotation_my_turn(
            &oracle_bytes,
            n_validators,
            config.interval_s,
        );

        if !is_my_turn {
            println!("[{}] ⟳ Not my turn (rotation window {})", now_str(), window_id);
            update_status_silent(SilentReason::NotElected, balance_xnt, days_left, &config, &oracle_pubkey);
            thread::sleep(Duration::from_secs(config.interval_s));
            continue;
        }

        // Get recent blockhash and send
        match rpc.get_recent_blockhash() {
            Err(e) => {
                eprintln!("[{}] ✗ Blockhash failed: {}", now_str(), e);
                update_status_silent(SilentReason::NoHealthyRpc, balance_xnt, days_left, &config, &oracle_pubkey);
                thread::sleep(Duration::from_secs(config.interval_s));
                continue;
            }
            Ok(blockhash) => {
                let tx = build_submit_transaction(
                    &keypair,
                    &program_id_arr,
                    &oracle_pda_arr,
                    &reg_pda_bytes,
                    &blockhash,
                    &consensus,
                    window_id,
                );

                let tx_b64 = base64_encode(&tx);

                match rpc.send_transaction(&tx_b64) {
                    Ok(sig) => {
                        let short_sig = shorten(&sig, 8);
                        println!("✅ submit OK — tx: {}", sig);
                        println!("[{}] ✓ TX: {}", now_str(), short_sig);
                        update_status_ok(&sig, &consensus, balance_xnt, days_left, &config, &oracle_pubkey, window_id);
                    }
                    Err(e) => {
                        eprintln!("[{}] ✗ Submit failed: {}", now_str(), e);
                        update_status_silent(SilentReason::TxRejected, balance_xnt, days_left, &config, &oracle_pubkey);
                    }
                }
            }
        }

        thread::sleep(Duration::from_secs(config.interval_s));
    }
}

// ─── Readiness Check ─────────────────────────────────────────────────────────

fn readiness_check(config: &StrontiumConfig, _oracle_pubkey: &str, _rpc: &mut RpcClient) {
    println!("[{}] ⏳ Waiting for validator to be ready...", now_str());

    let vote_path = match &config.vote_keypair_path {
        Some(p) => p.clone(),
        None => { return; } // No vote keypair configured, skip check
    };

    let vote_kp = match load_keypair(&vote_path) {
        Ok(kp) => kp,
        Err(_) => { return; } // Can't load, skip check
    };
    let vote_pubkey = bs58::encode(vote_kp.verifying_key().to_bytes()).into_string();

    for attempt in 0..READINESS_MAX_TRIES {
        if is_validator_active(&vote_pubkey) {
            println!("[{}] ✓ Validator is active — starting daemon", now_str());
            return;
        }

        if attempt == 0 {
            println!("  Validator not yet active in network. Waiting up to 20 minutes...");
        }

        println!("  [{}/{}] Checking validator status in 60s...", attempt + 1, READINESS_MAX_TRIES);
        thread::sleep(Duration::from_secs(60));
    }

    println!("[{}] ⚠ Validator readiness timeout — starting daemon anyway", now_str());
}

fn is_validator_active(vote_pubkey: &str) -> bool {
    let url = format!("https://api.x1.xyz/v1/validators/{}", vote_pubkey);
    match ureq::get(&url)
        .set("User-Agent", "X1-Strontium/1.0")
        .timeout(Duration::from_secs(10))
        .call()
    {
        Ok(resp) => {
            let body = resp.into_string().unwrap_or_default();
            body.contains("\"isActive\":true") || body.contains("\"status\":\"active\"")
        }
        Err(_) => false,
    }
}

// ─── Subcommands ─────────────────────────────────────────────────────────────

fn cmd_stop() {
    let status = DaemonStatus::load();
    match status.pid {
        Some(pid) => {
            println!("Stopping strontium daemon (PID {})...", pid);
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
            println!("Stopped.");
        }
        None => println!("Daemon is not running (no PID in status.json)"),
    }
}

fn cmd_status() {
    let status = DaemonStatus::load();
    status.print();
}

fn cmd_sources() {
    let status = DaemonStatus::load();
    if status.ntp_sources.is_empty() {
        println!("No source data available. Is the daemon running?");
        println!("Start with: x1sr start");
    } else {
        status.print_sources();
    }
}

fn cmd_history(args: &[String]) {
    let limit = args.first()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(10);

    println!("X1 Strontium — recent submissions (last {})", limit);
    println!("{}", "━".repeat(70));
    println!("  On-chain history is stored in Memo Program transactions.");
    println!("  View at: https://explorer.mainnet.x1.xyz/address/{}", ORACLE_PDA);
    println!();
    println!("  To export: x1sr archive --output ~/strontium_archive.jsonl");
}

fn cmd_config(args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        let config = StrontiumConfig::load();
        config.display();
        return;
    }

    if args[0] == "set" && args.len() >= 3 {
        let mut config = StrontiumConfig::load();
        match config.set(&args[1], &args[2]) {
            Ok(()) => {
                config.save().unwrap_or_else(|e| eprintln!("Save error: {}", e));
                println!("✓ Set {} = {}", args[1], args[2]);
            }
            Err(e) => eprintln!("✗ {}", e),
        }
        return;
    }

    eprintln!("Usage: x1sr config show");
    eprintln!("       x1sr config set <key> <value>");
    eprintln!("Keys: interval, rpc, keypair, vote_keypair, alert_webhook, alert_balance, dry_run");
}

fn cmd_deregister() {
    println!("Deregistration requires a transaction signed by your oracle keypair.");
    println!("This will close your ValidatorRegistration account and refund rent.");
    println!();
    println!("Not yet implemented in CLI. Coming in next release.");
    println!("You can deregister via: https://validator.x1.wiki");
}

fn cmd_balance() {
    let config = StrontiumConfig::load();
    let keypair = match load_keypair(&config.keypair_path) {
        Ok(kp) => kp,
        Err(e) => { eprintln!("✗ {}", e); return; }
    };
    let pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();
    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    match rpc.get_balance(&pubkey) {
        Ok(lamps) => {
            let xnt = lamports_to_xnt(lamps);
            let days = estimate_days_remaining(xnt, config.interval_s);
            println!("Oracle keypair balance:");
            println!("  Address  : {}", pubkey);
            println!("  Balance  : {:.3} XNT", xnt);
            println!("  Runway   : ~{:.0} days (at {}s interval)", days, config.interval_s);
            if xnt < MIN_BALANCE_WARN {
                println!("  ⚠ Balance is low — top up soon!");
            }
        }
        Err(e) => eprintln!("✗ Balance check failed: {}", e),
    }
}

fn cmd_archive(args: &[String]) {
    let output = args.windows(2)
        .find(|w| w[0] == "--output")
        .map(|w| w[1].as_str())
        .unwrap_or("~/strontium_archive.jsonl");

    println!("Archiving on-chain history...");
    println!("Output: {}", output);
    println!();
    println!("This feature reads historical Memo Program transactions from the oracle PDA.");
    println!("Full implementation coming in next release.");
    println!("For now, use X1 Explorer:");
    println!("  https://explorer.mainnet.x1.xyz/address/{}", ORACLE_PDA);
}

fn cmd_install() {
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "x1pio".to_string());

    let binary_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| format!("/home/{}/strontium", username));

    // Check keypair exists
    let config = StrontiumConfig::load();
    if !std::path::Path::new(&config.keypair_path).exists() {
        eprintln!("✗ Oracle keypair not found: {}", config.keypair_path);
        eprintln!("  Generate one first:");
        eprintln!("  mkdir -p ~/.config/strontium");
        eprintln!("  solana-keygen new --outfile ~/.config/strontium/oracle-keypair.json --no-bip39-passphrase");
        std::process::exit(1);
    }

    // Check balance
    println!("Checking oracle keypair balance...");
    let keypair = load_keypair(&config.keypair_path).expect("Failed to load keypair");
    let pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();
    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    let balance_xnt = match rpc.get_balance(&pubkey) {
        Ok(lamps) => lamports_to_xnt(lamps),
        Err(_) => 0.0,
    };

    let days = estimate_days_remaining(balance_xnt, config.interval_s);
    println!("  Balance: {:.3} XNT (~{:.0} days)", balance_xnt, days);

    if balance_xnt < MIN_BALANCE_STOP {
        eprintln!("⚠ Balance too low for operation ({:.3} XNT)", balance_xnt);
        eprintln!("  Minimum recommended: 1 XNT");
        eprintln!();
        eprintln!("  Fund your oracle keypair:");
        eprintln!("  solana transfer {} 1 --url https://rpc.mainnet.x1.xyz --keypair <YOUR_WALLET> --allow-unfunded-recipient",
            pubkey);
        eprintln!();
        print!("  Continue anyway? [y/N]: ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Installation cancelled.");
            return;
        }
    }

    println!();
    println!("X1 Strontium — installing as system service");
    println!("{}", "━".repeat(50));
    println!("  User        : {}", username);
    println!("  Binary      : {}", binary_path);
    println!("  Keypair     : {}", config.keypair_path);

    let service_content = format!(
        r#"[Unit]
Description=X1 Strontium Time Oracle Daemon
After=network.target
Wants=network-online.target

[Service]
Type=simple
User={user}
ExecStartPre=/bin/sleep 120
ExecStart={binary} start
Restart=on-failure
RestartSec=30
StandardOutput=append:/home/{user}/strontium.log
StandardError=append:/home/{user}/strontium.log

[Install]
WantedBy=multi-user.target
"#,
        user = username,
        binary = binary_path
    );

    let service_path = "/etc/systemd/system/strontium.service";
    match std::fs::write(service_path, &service_content) {
        Ok(()) => println!("  Service     : {} ✓", service_path),
        Err(e) => {
            eprintln!("✗ Cannot write service file: {}", e);
            eprintln!("  Try: sudo x1sr install");
            return;
        }
    }

    // Create x1sr alias
    let alias_path = "/usr/local/bin/x1sr";
    let alias_content = format!("#!/bin/sh\nexec {} \"$@\"\n", binary_path);
    match std::fs::write(alias_path, &alias_content) {
        Ok(()) => {
            let _ = std::process::Command::new("chmod").args(["+x", alias_path]).status();
            println!("  Alias x1sr  : {} ✓", alias_path);
        }
        Err(_) => eprintln!("  ⚠ Could not create x1sr alias (try sudo)"),
    }

    // Enable and start
    let _ = std::process::Command::new("systemctl")
        .args(["daemon-reload"]).status();
    let _ = std::process::Command::new("systemctl")
        .args(["enable", "strontium"]).status();
    let enable_ok = std::process::Command::new("systemctl")
        .args(["start", "strontium"]).status()
        .map(|s| s.success()).unwrap_or(false);

    println!("  Autostart   : enabled ✓");
    if enable_ok {
        println!("  Status      : started ✓");
    }

    println!();
    println!("✓ Strontium will now start automatically on boot.");
    println!("  Check status: x1sr status");
    println!("  View logs   : tail -f ~/strontium.log");
}

fn cmd_uninstall() {
    println!("Uninstalling X1 Strontium service...");
    let _ = std::process::Command::new("systemctl").args(["stop", "strontium"]).status();
    let _ = std::process::Command::new("systemctl").args(["disable", "strontium"]).status();
    let _ = std::fs::remove_file("/etc/systemd/system/strontium.service");
    let _ = std::fs::remove_file("/usr/local/bin/x1sr");
    let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
    println!("✓ Strontium service removed.");
    println!("  Config and keypair are preserved in ~/.config/strontium/");
}

// ─── Status Updates ───────────────────────────────────────────────────────────

fn update_status_ok(
    sig: &str,
    consensus: &consensus::ConsensusResult,
    balance: f64,
    days: f64,
    config: &StrontiumConfig,
    pubkey: &str,
    window_id: u64,
) {
    let mut s = DaemonStatus::load();
    s.running          = true;
    s.pid              = Some(std::process::id());
    s.last_submit_ts   = Some(consensus.timestamp_ms);
    s.last_submit_tx   = Some(sig.to_string());
    s.last_attempt_ts  = Some(unix_ms());
    s.silent_cycles    = 0;
    s.silent_reason    = None;
    s.balance_xnt      = balance;
    s.days_remaining   = days;
    s.balance_warning  = balance < MIN_BALANCE_WARN;
    s.interval_s       = config.interval_s;
    s.dry_run          = config.dry_run;
    s.consensus_ms     = Some(consensus.timestamp_ms);
    s.spread_ms        = Some(consensus.spread_ms);
    s.confidence       = Some(consensus.confidence);
    s.sources_bitmap   = consensus.sources_bitmap;
    s.oracle_pubkey    = Some(pubkey.to_string());
    s.rotation_window_id = Some(window_id);
    s.rotation_is_my_turn = Some(true);
    s.last_error       = None;
    s.save();
}

fn update_status_silent(
    reason: SilentReason,
    balance: f64,
    days: f64,
    config: &StrontiumConfig,
    pubkey: &str,
) {
    let mut s = DaemonStatus::load();
    s.running         = true;
    s.pid             = Some(std::process::id());
    s.last_attempt_ts = Some(unix_ms());
    s.silent_cycles   = s.silent_cycles.saturating_add(1);
    s.silent_reason   = Some(reason);
    s.balance_xnt     = balance;
    s.days_remaining  = days;
    s.balance_warning = balance < MIN_BALANCE_WARN;
    s.interval_s      = config.interval_s;
    s.dry_run         = config.dry_run;
    s.oracle_pubkey   = Some(pubkey.to_string());
    s.save();

    if s.silent_cycles >= 3 {
        eprintln!("[warn] Silent for {} cycles: {}",
            s.silent_cycles,
            s.silent_reason.as_ref().map(|r| r.to_string()).unwrap_or_default());
    }
}

fn update_status_dry_run(
    consensus: &consensus::ConsensusResult,
    balance: f64,
    days: f64,
    config: &StrontiumConfig,
    pubkey: &str,
) {
    let mut s = DaemonStatus::load();
    s.running         = true;
    s.pid             = Some(std::process::id());
    s.last_attempt_ts = Some(unix_ms());
    s.silent_cycles   = s.silent_cycles.saturating_add(1);
    s.silent_reason   = Some(SilentReason::DryRun);
    s.balance_xnt     = balance;
    s.days_remaining  = days;
    s.interval_s      = config.interval_s;
    s.dry_run         = true;
    s.consensus_ms    = Some(consensus.timestamp_ms);
    s.spread_ms       = Some(consensus.spread_ms);
    s.confidence      = Some(consensus.confidence);
    s.sources_bitmap  = consensus.sources_bitmap;
    s.oracle_pubkey   = Some(pubkey.to_string());
    s.save();
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn apply_cli_overrides(config: &mut StrontiumConfig, args: &[String]) {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--keypair" if i + 1 < args.len() => {
                config.keypair_path = args[i+1].clone(); i += 2;
            }
            "--vote-keypair" if i + 1 < args.len() => {
                config.vote_keypair_path = Some(args[i+1].clone()); i += 2;
            }
            "--rpc" if i + 1 < args.len() => {
                config.rpc_urls.insert(0, args[i+1].clone()); i += 2;
            }
            "--interval" if i + 1 < args.len() => {
                if let Ok(v) = args[i+1].parse() { config.interval_s = v; } i += 2;
            }
            "--dry-run" => { config.dry_run = true; i += 1; }
            "--program" if i + 1 < args.len() => { i += 2; } // ignored, use config
            "--pda"     if i + 1 < args.len() => { i += 2; } // ignored, use config
            _ => { i += 1; }
        }
    }
}

fn check_balance(pubkey: &str, rpc: &mut RpcClient) -> f64 {
    rpc.get_balance(pubkey)
        .map(lamports_to_xnt)
        .unwrap_or(0.0)
}

fn print_ntp_sources(sources: &[ntp_client::NtpResult]) {
    println!("[{}] ✓ Selected servers ({}):", now_str(), sources.len());
    for s in sources {
        println!("[{}]   {} ({})", now_str(), s.host,
            match &s.tier {
                status::NtpTier::Gps      => "GPS",
                status::NtpTier::Nts      => "NTS",
                status::NtpTier::Stratum1 => "Stratum-1",
                status::NtpTier::Pool     => "Pool",
            });
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn now_str() -> String {
    let ms = unix_ms();
    let s  = ms / 1000;
    let h  = (s % 86400) / 3600;
    let m  = (s % 3600) / 60;
    let sc = s % 60;
    format!("{:02}:{:02}:{:02}", h, m, sc)
}

fn format_ts(ms: i64) -> String {
    let s = ms / 1000;
    let h = (s % 86400) / 3600;
    let m = (s % 3600) / 60;
    let sc = s % 60;
    let frac = ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03} UTC", h, m, sc, frac)
}

fn shorten(s: &str, n: usize) -> String {
    if s.len() <= n * 2 + 3 { return s.to_string(); }
    format!("{}...{}", &s[..n], &s[s.len()-n..])
}

fn base64_encode(data: &[u8]) -> String {
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

// ─── Help ─────────────────────────────────────────────────────────────────────

fn print_help() {
    println!(r#"X1 Strontium — decentralized time oracle for X1 blockchain

USAGE:
  strontium <command> [options]
  x1sr <command> [options]

COMMANDS:
  start              Start the daemon
  stop               Stop the running daemon
  status             Show daemon status and NTP consensus
  sources            Show NTP source details
  history [n]        Show last N submissions (default: 10)
  config show        Show current configuration
  config set <k> <v> Set a config value
  register           Register validator (one-time setup)
  deregister         Remove registration
  balance            Show oracle keypair balance and runway
  archive            Export on-chain history
  install            Install as systemd service (requires sudo)
  uninstall          Remove systemd service

OPTIONS (for start):
  --keypair <path>      Oracle keypair path (default: ~/.config/strontium/oracle-keypair.json)
  --vote-keypair <path> Vote keypair path (default: ~/.config/solana/vote.json)
  --interval <secs>     Submit interval (default: 300)
  --rpc <url>           RPC endpoint (prepended to fallback list)
  --dry-run             Run without submitting transactions

CONFIG KEYS:
  interval, rpc, keypair, vote_keypair, alert_webhook, alert_balance, dry_run

GENERATE KEYPAIR:
  mkdir -p ~/.config/strontium
  solana-keygen new --outfile ~/.config/strontium/oracle-keypair.json --no-bip39-passphrase

QUICK START:
  1. Generate oracle keypair (above)
  2. Fund it: solana transfer <ORACLE_PUBKEY> 1 --url https://rpc.mainnet.x1.xyz --keypair <WALLET>
  3. Register: x1sr register
  4. Start:    x1sr start
  5. Install:  sudo x1sr install

EXPLORER:
  Oracle PDA: https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn
"#);
}
