mod config;
mod status;
mod ntp_client;
mod consensus;
mod submitter;
mod register;
mod rotation;

use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config::StrontiumConfig;
use status::{DaemonStatus, SilentReason};
use ntp_client::{discover_sources, query_ntp, to_source_status, has_gps_pps};
use consensus::run_consensus_cycle;
use submitter::{
    RpcClient, build_submit_transaction_signed, SubmitParams,
    derive_registration_pda, lamports_to_xnt, estimate_days_remaining, base64_encode,
};
use register::load_keypair;
use rotation::{rotation_my_turn, window_has_submission, RotationState};

// ── Constants ─────────────────────────────────────────────────────────────────
const MIN_BALANCE_WARN:   f64 = 1.0;
const MIN_BALANCE_STOP:   f64 = 0.05;
const REDISCOVER_SECS:    u64 = 3600;
const READINESS_MAX_TRIES: u32 = 20;

// ── Entry point ───────────────────────────────────────────────────────────────
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "help" || args[1] == "-h" {
        print_help();
        return;
    }

    match args[1].as_str() {
        "start" => {
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[2..]);
            run_daemon(config);
        }
        "stop"       => cmd_stop(),
        "status"     => cmd_status(),
        "sources"    => cmd_sources(),
        "history"    => cmd_history(&args[2..]),
        "config"     => cmd_config(&args[2..]),
        "register"   => {
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[2..]);
            if let Err(e) = register::run_register(&mut config) {
                eprintln!("✗ {}", e);
                std::process::exit(1);
            }
        }
        "deregister" => cmd_deregister(),
        "balance"    => cmd_balance(),
        "archive"    => cmd_archive(&args[2..]),
        "install"    => cmd_install(),
        "uninstall"  => cmd_uninstall(),
        _ => {
            // Legacy mode: strontium --keypair ...
            let mut config = StrontiumConfig::load();
            apply_cli_overrides(&mut config, &args[1..]);
            run_daemon(config);
        }
    }
}

// ── Main daemon loop ──────────────────────────────────────────────────────────
fn run_daemon(config: StrontiumConfig) {
    println!("╔═══════════════════════════════════════╗");
    println!("║   X1 Strontium — Time Oracle Daemon  ║");
    println!("╚═══════════════════════════════════════╝");

    let keypair = match load_keypair(&config.keypair_path) {
        Ok(kp) => kp,
        Err(e) => { eprintln!("✗ Keypair error: {}", e); std::process::exit(1); }
    };

    let oracle_pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();
    let oracle_bytes:  [u8; 32] = keypair.verifying_key().to_bytes();

    let program_id_bytes = bs58::decode(&config.program_id).into_vec().expect("Invalid program ID");
    let program_id_arr:  [u8; 32] = program_id_bytes.try_into().expect("Program ID wrong length");

    let oracle_pda_bytes = bs58::decode(&config.oracle_pda).into_vec().expect("Invalid oracle PDA");
    let oracle_pda_arr:  [u8; 32] = oracle_pda_bytes.try_into().expect("Oracle PDA wrong length");

    let reg_pda = derive_registration_pda(&oracle_bytes, &program_id_arr);

    println!("  Keypair   : {}", oracle_pubkey);
    println!("  Program   : {}", config.program_id);
    println!("  PDA       : {}", config.oracle_pda);
    println!("  Interval  : {}s", config.interval_s);
    println!("  Mode      : {}", if config.dry_run { "DRY-RUN (no tx sent)" } else { "live" });
    if has_gps_pps() {
        println!("  GPS/PPS   : detected (/dev/pps0) — using as tier-0 source");
    }

    let mut rpc = RpcClient::new(config.rpc_urls.clone());

    // Readiness check — wait for validator to be active
    if !config.dry_run {
        readiness_check(&config, &oracle_pubkey);
    }

    // Initial balance check
    let balance_xnt = check_balance(&oracle_pubkey, &mut rpc);
    if balance_xnt < MIN_BALANCE_STOP {
        eprintln!("✗ Balance too low: {:.3} XNT", balance_xnt);
        eprintln!("  Fund your oracle keypair:");
        eprintln!("  solana transfer {} 1 --url {} --keypair <YOUR_WALLET> --allow-unfunded-recipient",
            oracle_pubkey, config.rpc_urls.first().map(|s| s.as_str()).unwrap_or("https://rpc.mainnet.x1.xyz"));
        std::process::exit(1);
    }

    // Discover NTP sources
    println!("[{}] 🔍 Discovering NTP servers...", now_str());
    let mut ntp_cache = discover_sources(3);
    let mut last_rediscover = unix_secs();
    let mut cycle_num = 0u64;

    // Rotation state
    let mut rotation = RotationState::new();
    rotation.active_oracles.push(oracle_bytes); // start with self

    print_ntp_sources(&ntp_cache);

    loop {
        cycle_num += 1;

        let balance_xnt = check_balance(&oracle_pubkey, &mut rpc);
        let days_left   = estimate_days_remaining(balance_xnt, config.interval_s);

        // N5: Critical balance check first
        if balance_xnt < MIN_BALANCE_STOP {
            eprintln!("✗ Balance critically low: {:.3} XNT — stopping daemon", balance_xnt);
            let mut st = DaemonStatus::load();
            st.set_silent_reason(SilentReason::InsufficientBalance);
            st.save();
            std::process::exit(1);
        }
        if balance_xnt < MIN_BALANCE_WARN {
            eprintln!("[warn] Balance low: {:.3} XNT (~{:.0} days remaining)", balance_xnt, days_left);
            // P11: Send alert webhook
            if let Some(ref webhook) = config.alert_webhook {
                send_alert_webhook(webhook, &format!(
                    "⚠️ X1 Strontium balance low: {:.3} XNT (~{:.0} days) — oracle: {}",
                    balance_xnt, days_left, &oracle_pubkey[..8]
                ));
            }
        }

        // Re-discover NTP sources periodically
        if unix_secs() - last_rediscover > REDISCOVER_SECS {
            println!("[{}] 🔍 Re-discovering NTP servers...", now_str());
            ntp_cache = discover_sources(3);
            print_ntp_sources(&ntp_cache);
            last_rediscover = unix_secs();
        }

        // Query NTP sources in parallel
        let results: Vec<_> = {
            use std::sync::{Arc, Mutex};
            let collected = Arc::new(Mutex::new(Vec::new()));
            let mut handles = Vec::new();
            for src in &ntp_cache {
                let host    = src.host.clone();
                let tier    = src.tier.clone();
                let stratum = src.stratum;
                let col     = Arc::clone(&collected);
                let h = thread::spawn(move || {
                    if let Some(r) = query_ntp(&host, 123, tier, stratum) {
                        col.lock().unwrap().push(r);
                    }
                });
                handles.push(h);
            }
            for h in handles { let _ = h.join(); }
            collected.lock().unwrap().clone()
        };

        println!("[{}] Cycle #{}: {} measurements from {} servers",
            now_str(), cycle_num, results.len(), ntp_cache.len());
        for r in &results {
            println!("  {:<32} rtt={:4}ms  offset={:+6}ms  stratum={}",
                r.host, r.rtt_ms, r.offset_ms, r.stratum);
        }

        // Compute consensus
        let consensus = match run_consensus_cycle(&results, config.tier_consensus_threshold_ms) {
            Some(c) => c,
            None => {
                println!("[{}] ✗ No consensus (spread too high or insufficient sources)", now_str());
                update_status_silent(SilentReason::SpreadTooHigh, balance_xnt, days_left,
                    &config, &oracle_pubkey, &results);
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

        if config.dry_run {
            println!("[{}] ⚠ [DRY-RUN] tx skipped", now_str());
            update_status_dry_run(&consensus, balance_xnt, days_left, &config, &oracle_pubkey, &results);
            thread::sleep(Duration::from_secs(config.interval_s));
            continue;
        }

        // Rotation check — N3: no solo mode, rotation always applies (n=1 → always my turn)
        let n_oracles  = rotation.n_oracles();
        let my_index   = rotation.my_index(&oracle_bytes);
        let (is_my_turn, window_id, _) = rotation_my_turn(
            &oracle_bytes, my_index, n_oracles, config.interval_s,
        );

        if !is_my_turn {
            let st = DaemonStatus::load();
            if window_has_submission(st.last_submit_ts, config.interval_s) {
                println!("[{}] ⟳ Window {} already covered", now_str(), window_id);
            } else {
                println!("[{}] ⟳ Not my turn (window {})", now_str(), window_id);
            }
            update_status_silent(SilentReason::NotElected, balance_xnt, days_left,
                &config, &oracle_pubkey, &results);
            thread::sleep(Duration::from_secs(config.interval_s));
            continue;
        }

        // Outlier check vs SYSTEM CLOCK (not chain clock — N5 fix)
        let sys_ms  = unix_ms();
        let sys_drift = (consensus.timestamp_ms - sys_ms).abs();
        if sys_drift > 5_000 {
            eprintln!("[{}] ✗ Timestamp outlier: NTP {}ms vs system {}ms (drift={}ms)",
                now_str(), consensus.timestamp_ms, sys_ms, sys_drift);
            update_status_silent(SilentReason::TimestampOutlier, balance_xnt, days_left,
                &config, &oracle_pubkey, &results);
            thread::sleep(Duration::from_secs(config.interval_s));
            continue;
        }

        // Get chain time for Memo only (P8)
        let chain_time_ms = rpc.get_chain_time_ms();

        // Get blockhash
        let blockhash = match rpc.get_recent_blockhash() {
            Err(e) => {
                eprintln!("[{}] ✗ Blockhash failed: {}", now_str(), e);
                update_status_silent(SilentReason::NoHealthyRpc, balance_xnt, days_left,
                    &config, &oracle_pubkey, &results);
                thread::sleep(Duration::from_secs(config.interval_s));
                continue;
            }
            Ok(bh) => bh,
        };

        // Build and send TX
        let tx = build_submit_transaction_signed(
            &keypair, &program_id_arr, &oracle_pda_arr, &reg_pda, &blockhash,
            &SubmitParams {
                consensus:     &consensus,
                window_id,
                memo_enabled:  config.memo_enabled,
                chain_time_ms,
            },
        );
        let tx_b64 = base64_encode(&tx);

        match rpc.send_transaction(&tx_b64) {
            Ok(sig) => {
                println!("✅ submit OK — tx: {}", sig);
                println!("[{}] ✓ TX: {}...{}", now_str(), &sig[..8], &sig[sig.len()-8..]);
                update_status_ok(&sig, &consensus, balance_xnt, days_left,
                    &config, &oracle_pubkey, window_id, &results);
            }
            Err(e) => {
                eprintln!("[{}] ✗ Submit failed: {}", now_str(), e);
                let mut st = DaemonStatus::load();
                st.set_silent_reason(SilentReason::TxRejected); // N5: won't override InsufficientBalance
                st.save();
            }
        }

        thread::sleep(Duration::from_secs(config.interval_s));
    }
}

// ── Readiness check ───────────────────────────────────────────────────────────
fn readiness_check(config: &StrontiumConfig, oracle_pubkey: &str) {
    println!("[{}] ⏳ Waiting for validator to be ready...", now_str());

    let vote_path = match &config.vote_keypair_path {
        Some(p) => p.clone(),
        None => return,
    };
    let vote_kp = match load_keypair(&vote_path) {
        Ok(kp) => kp,
        Err(_) => return,
    };

    // Use oracle keypair pubkey for API check (identity-based)
    let check_pubkey = oracle_pubkey.to_string();

    for attempt in 0..READINESS_MAX_TRIES {
        if is_validator_active(&check_pubkey) {
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

fn is_validator_active(pubkey: &str) -> bool {
    let url = format!("https://api.x1.xyz/v1/validators/{}", pubkey);
    match ureq::get(&url)
        .set("User-Agent", "X1-Strontium/1.0")
        .timeout(Duration::from_secs(10))
        .call()
    {
        Ok(resp) => {
            let body = resp.into_string().unwrap_or_default();
            body.contains("\"active\":true")
        }
        Err(_) => false,
    }
}

// ── Subcommands ───────────────────────────────────────────────────────────────

fn cmd_stop() {
    let st = DaemonStatus::load();
    match st.pid {
        Some(pid) => {
            println!("Stopping strontium daemon (PID {})...", pid);
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
            println!("Stopped.");
        }
        None => println!("Daemon is not running (no PID in status.json)"),
    }
}

fn cmd_status() {
    DaemonStatus::load().print();
}

fn cmd_sources() {
    let st = DaemonStatus::load();
    if st.ntp_sources.is_empty() {
        println!("No source data available. Is the daemon running?");
        println!("Start with: x1sr start");
    } else {
        st.print_sources();
    }
}

fn cmd_history(args: &[String]) {
    let limit = args.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
    println!("X1 Strontium — recent submissions (last {})", limit);
    println!("{}", "━".repeat(70));
    println!("  On-chain history is stored in Memo Program transactions.");
    println!("  View at: https://explorer.mainnet.x1.xyz/address/{}", config::ORACLE_PDA);
    println!();
    println!("  To export: x1sr archive --output ~/strontium_archive.jsonl");
}

fn cmd_config(args: &[String]) {
    if args.is_empty() || args[0] == "show" {
        StrontiumConfig::load().display();
        return;
    }
    if args[0] == "set" && args.len() >= 3 {
        let mut cfg = StrontiumConfig::load();
        match cfg.set(&args[1], &args[2]) {
            Ok(()) => {
                cfg.save().unwrap_or_else(|e| eprintln!("Save error: {}", e));
                println!("✓ Set {} = {}", args[1], args[2]);
            }
            Err(e) => eprintln!("✗ {}", e),
        }
        return;
    }
    eprintln!("Usage: x1sr config show");
    eprintln!("       x1sr config set <key> <value>");
}

fn cmd_deregister() {
    println!("Deregistration closes your ValidatorRegistration PDA and returns rent.");
    println!("Not yet implemented in CLI.");
    println!("Coming in next release.");
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
            let xnt  = lamports_to_xnt(lamps);
            let days = estimate_days_remaining(xnt, config.interval_s);
            println!("Oracle keypair balance:");
            println!("  Address  : {}", pubkey);
            println!("  Balance  : {:.3} XNT", xnt);
            println!("  Runway   : ~{:.0} days (at {}s interval)", days, config.interval_s);
            if xnt < MIN_BALANCE_WARN { println!("  ⚠ Balance is low — top up soon!"); }
        }
        Err(e) => eprintln!("✗ Balance check failed: {}", e),
    }
}

fn cmd_archive(args: &[String]) {
    let output = args.windows(2)
        .find(|w| w[0] == "--output")
        .map(|w| w[1].as_str())
        .unwrap_or("~/strontium_archive.jsonl");
    println!("Archiving on-chain history to {}...", output);
    println!("Full implementation in next release.");
    println!("View at: https://explorer.mainnet.x1.xyz/address/{}", config::ORACLE_PDA);
}

fn cmd_install() {
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "x1pio".to_string());
    let binary_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| format!("/usr/local/bin/strontium"));

    let config = StrontiumConfig::load();
    let keypair = match load_keypair(&config.keypair_path) {
        Ok(kp) => kp,
        Err(e) => { eprintln!("✗ Oracle keypair not found: {}\n  Generate: solana-keygen new --outfile ~/.config/strontium/oracle-keypair.json --no-bip39-passphrase", e); std::process::exit(1); }
    };
    let pubkey = bs58::encode(keypair.verifying_key().to_bytes()).into_string();
    let mut rpc = RpcClient::new(config.rpc_urls.clone());
    let xnt = rpc.get_balance(&pubkey).map(lamports_to_xnt).unwrap_or(0.0);
    println!("  Balance: {:.3} XNT", xnt);

    // N4: No sleep — readiness check handles startup timing
    let service = format!(r#"[Unit]
Description=X1 Strontium Time Oracle Daemon
After=network.target
Wants=network-online.target

[Service]
Type=simple
User={user}
ExecStart={binary} start
Restart=on-failure
RestartSec=30
StandardOutput=append:/home/{user}/strontium.log
StandardError=append:/home/{user}/strontium.log

[Install]
WantedBy=multi-user.target
"#, user = username, binary = binary_path);

    let path = "/etc/systemd/system/strontium.service";
    match std::fs::write(path, &service) {
        Ok(()) => println!("  Service: {} ✓", path),
        Err(e) => { eprintln!("✗ Cannot write service: {}\n  Try: sudo x1sr install", e); return; }
    }

    // Create x1sr symlink
    let _ = std::fs::write("/usr/local/bin/x1sr",
        format!("#!/bin/sh\nexec {} \"$@\"\n", binary_path));
    let _ = std::process::Command::new("chmod").args(["+x", "/usr/local/bin/x1sr"]).status();

    let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
    let _ = std::process::Command::new("systemctl").args(["enable", "strontium"]).status();
    let _ = std::process::Command::new("systemctl").args(["start", "strontium"]).status();

    println!("✓ Strontium installed and started.");
    println!("  Check status: x1sr status");
    println!("  View logs   : tail -f ~/strontium.log");
}

fn cmd_uninstall() {
    let _ = std::process::Command::new("systemctl").args(["stop", "strontium"]).status();
    let _ = std::process::Command::new("systemctl").args(["disable", "strontium"]).status();
    let _ = std::fs::remove_file("/etc/systemd/system/strontium.service");
    let _ = std::fs::remove_file("/usr/local/bin/x1sr");
    let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
    println!("✓ Strontium service removed.");
    println!("  Config and keypair preserved in ~/.config/strontium/");
}

// ── Status updates ────────────────────────────────────────────────────────────

fn update_status_ok(
    sig: &str, consensus: &consensus::ConsensusResult,
    balance: f64, days: f64, config: &StrontiumConfig,
    pubkey: &str, window_id: u64,
    sources: &[ntp_client::NtpResult],
) {
    let mut st = DaemonStatus::load();
    st.running           = true;
    st.pid               = Some(std::process::id());
    st.last_submit_ts    = Some(consensus.timestamp_ms);
    st.last_submit_tx    = Some(sig.to_string());
    st.last_attempt_ts   = Some(unix_ms());
    st.silent_cycles     = 0;
    st.silent_reason     = None;
    st.balance_xnt       = balance;
    st.days_remaining    = days;
    st.balance_warning   = balance < MIN_BALANCE_WARN;
    st.interval_s        = config.interval_s;
    st.dry_run           = config.dry_run;
    st.consensus_ms      = Some(consensus.timestamp_ms);
    st.spread_ms         = Some(consensus.spread_ms);
    st.confidence        = Some(consensus.confidence);
    st.sources_bitmap    = consensus.sources_bitmap;
    st.oracle_pubkey     = Some(pubkey.to_string());
    st.rotation_window_id = Some(window_id);
    st.rotation_is_my_turn = Some(true);
    st.ntp_sources       = to_source_status(sources);
    st.last_error        = None;
    st.save();
}

fn update_status_silent(
    reason: SilentReason, balance: f64, days: f64,
    config: &StrontiumConfig, pubkey: &str,
    sources: &[ntp_client::NtpResult],
) {
    let mut st = DaemonStatus::load();
    st.running         = true;
    st.pid             = Some(std::process::id());
    st.last_attempt_ts = Some(unix_ms());
    st.silent_cycles   = st.silent_cycles.saturating_add(1);
    st.set_silent_reason(reason.clone()); // N5: respects priority
    st.balance_xnt     = balance;
    st.days_remaining  = days;
    st.balance_warning = balance < MIN_BALANCE_WARN;
    st.interval_s      = config.interval_s;
    st.dry_run         = config.dry_run;
    st.oracle_pubkey   = Some(pubkey.to_string());
    st.ntp_sources     = to_source_status(sources);
    st.save();

    if st.silent_cycles >= 3 {
        let reason_str = reason.to_string();
        eprintln!("[warn] Silent for {} cycles: {}", st.silent_cycles, reason_str);

        // P11: Alert webhook for prolonged silence
        if st.silent_cycles == 3 || st.silent_cycles % 10 == 0 {
            if let Some(ref webhook) = config.alert_webhook {
                send_alert_webhook(webhook, &format!(
                    "⚠️ X1 Strontium silent for {} cycles: {} — oracle: {}",
                    st.silent_cycles, reason_str,
                    pubkey.get(..8).unwrap_or(pubkey)
                ));
            }
        }
    }
}

fn update_status_dry_run(
    consensus: &consensus::ConsensusResult, balance: f64, days: f64,
    config: &StrontiumConfig, pubkey: &str,
    sources: &[ntp_client::NtpResult],
) {
    let mut st = DaemonStatus::load();
    st.running         = true;
    st.pid             = Some(std::process::id());
    st.last_attempt_ts = Some(unix_ms());
    st.silent_cycles   = st.silent_cycles.saturating_add(1);
    st.set_silent_reason(SilentReason::DryRun);
    st.balance_xnt     = balance;
    st.days_remaining  = days;
    st.interval_s      = config.interval_s;
    st.dry_run         = true;
    st.consensus_ms    = Some(consensus.timestamp_ms);
    st.spread_ms       = Some(consensus.spread_ms);
    st.confidence      = Some(consensus.confidence);
    st.sources_bitmap  = consensus.sources_bitmap;
    st.oracle_pubkey   = Some(pubkey.to_string());
    st.ntp_sources     = to_source_status(sources);
    st.save();
}

// P11: Alert webhook
fn send_alert_webhook(url: &str, message: &str) {
    let body = serde_json::json!({ "text": message });
    let _ = ureq::post(url)
        .set("Content-Type", "application/json")
        .set("User-Agent", "X1-Strontium/1.0")
        .timeout(Duration::from_secs(10))
        .send_string(&body.to_string());
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
            _ => { i += 1; }
        }
    }
}

fn check_balance(pubkey: &str, rpc: &mut RpcClient) -> f64 {
    rpc.get_balance(pubkey).map(lamports_to_xnt).unwrap_or(0.0)
}

fn print_ntp_sources(sources: &[ntp_client::NtpResult]) {
    println!("[{}] ✓ Selected servers ({}):", now_str(), sources.len());
    for s in sources {
        println!("[{}]   {} ({:?})", now_str(), s.host, s.tier);
    }
}

fn unix_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

fn now_str() -> String {
    let ms = unix_ms();
    let s  = ms / 1000;
    format!("{:02}:{:02}:{:02}", (s%86400)/3600, (s%3600)/60, s%60)
}

fn format_ts(ms: i64) -> String {
    let s    = ms / 1000;
    let frac = ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03} UTC", (s%86400)/3600, (s%3600)/60, s%60, frac)
}

fn print_help() {
    println!(r#"X1 Strontium — decentralized time oracle for X1 blockchain

USAGE:
  strontium <command> [options]
  x1sr <command> [options]

COMMANDS:
  start              Start the daemon (live mode)
  start --dry-run    Start in test mode (no transactions)
  stop               Stop the running daemon
  status             Show daemon status and NTP consensus
  sources            Show NTP source details
  history [n]        Show last N on-chain submissions (default: 10)
  register           Register validator oracle (one-time setup)
  deregister         Deregister and recover rent
  balance            Show oracle keypair balance and runway
  archive            Export on-chain history to JSONL
  config show        Show current configuration
  config set <k> <v> Set a configuration value
  install            Install as systemd service (requires sudo)
  uninstall          Remove systemd service

CONFIG KEYS:
  interval           Submit interval in seconds (default: 300)
  keypair            Oracle keypair path
  vote_keypair       Vote keypair path
  rpc                Add RPC endpoint
  dry_run            Test mode (true/false)
  memo               Include Memo in TX (true/false)
  alert_webhook      Webhook URL for Telegram/Discord/Slack alerts
  alert_balance      Balance threshold for alerts (XNT)
  tier_threshold     Cross-tier consensus threshold (ms)

EXPLORER:
  Oracle PDA: https://explorer.mainnet.x1.xyz/address/EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn
"#);
}
