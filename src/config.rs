use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

pub const PROGRAM_ID:  &str = "2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe";
pub const ORACLE_PDA:  &str = "EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn";
pub const DEFAULT_INTERVAL_S: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrontiumConfig {
    /// Path to oracle keypair JSON
    pub keypair_path: String,

    /// Path to vote keypair JSON (for TTL renewal)
    pub vote_keypair_path: Option<String>,

    /// Submit interval in seconds (default: 300)
    pub interval_s: u64,

    /// RPC endpoints with automatic failover
    pub rpc_urls: Vec<String>,

    /// Program ID (hardcoded default)
    pub program_id: String,

    /// Oracle PDA (hardcoded default)
    pub oracle_pda: String,

    /// Optional webhook URL for alerts (e.g. ntfy.sh)
    pub alert_webhook: Option<String>,

    /// Alert when balance drops below this threshold (XNT)
    pub alert_balance_threshold: f64,

    /// Dry run mode — NTP consensus without submitting TX
    pub dry_run: bool,

    /// Cross-tier consensus threshold in ms (default: 60)
    /// At least one T-1 or T-2 source must agree with median within this window
    /// Based on real RTT measurements from X1 validator network
    pub tier_consensus_threshold_ms: i64,

    /// Include Memo Program instruction in each TX (default: true)
    /// Set to false for lower compute units in production
    pub memo_enabled: bool,

    /// Committee: sorted list of oracle pubkeys for rotation
    /// Empty = solo mode (always my turn)
    pub committee: Vec<String>,
}

impl Default for StrontiumConfig {
    fn default() -> Self {
        Self {
            keypair_path: default_keypair_path(),
            vote_keypair_path: find_default_vote_keypair(),
            interval_s: DEFAULT_INTERVAL_S,
            rpc_urls: vec![
                "http://localhost:8899".to_string(),
                "https://rpc.mainnet.x1.xyz".to_string(),
                "https://api.mainnet.x1.xyz".to_string(),
            ],
            program_id: PROGRAM_ID.to_string(),
            oracle_pda: ORACLE_PDA.to_string(),
            alert_webhook: None,
            alert_balance_threshold: 1.0,
            dry_run: false,
            committee: vec![],
            memo_enabled: true,
            tier_consensus_threshold_ms: 60,
        }
    }
}

impl StrontiumConfig {
    /// Load config from ~/.config/strontium/config.json
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    /// Save config to ~/.config/strontium/config.json
    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create config dir: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;
        fs::write(&path, json)
            .map_err(|e| format!("Write config error: {}", e))?;
        Ok(())
    }

    /// Set a config value by key
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "interval" | "interval_s" => {
                self.interval_s = value.parse::<u64>()
                    .map_err(|_| "interval must be a number in seconds".to_string())?;
            }
            "rpc" => {
                self.rpc_urls.insert(0, value.to_string());
            }
            "keypair" => {
                self.keypair_path = value.to_string();
            }
            "vote_keypair" => {
                self.vote_keypair_path = Some(value.to_string());
            }
            "alert_webhook" => {
                self.alert_webhook = Some(value.to_string());
            }
            "alert_balance" => {
                self.alert_balance_threshold = value.parse::<f64>()
                    .map_err(|_| "alert_balance must be a number".to_string())?;
            }
            "dry_run" => {
                self.dry_run = value == "true" || value == "1";
            }
            "committee" => {
                // Add a pubkey to committee list
                if !self.committee.contains(&value.to_string()) {
                    self.committee.push(value.to_string());
                    self.committee.sort();
                }
            }
            "committee_clear" => {
                self.committee.clear();
            }
            "tier_threshold" | "tier_consensus_threshold_ms" => {
                self.tier_consensus_threshold_ms = value.parse::<i64>()
                    .map_err(|_| "tier_threshold must be a number in ms".to_string())?;
            }
            "memo" | "memo_enabled" => {
                self.memo_enabled = value == "true" || value == "1";
            }
            _ => return Err(format!("Unknown config key: {}", key)),
        }
        Ok(())
    }

    /// Display config in human-readable format
    pub fn display(&self) {
        println!("X1 Strontium — configuration");
        println!("{}", "━".repeat(50));
        println!("  keypair        : {}", self.keypair_path);
        println!("  vote_keypair   : {}", self.vote_keypair_path.as_deref().unwrap_or("not set"));
        println!("  interval       : {}s", self.interval_s);
        println!("  program_id     : {}", self.program_id);
        println!("  oracle_pda     : {}", self.oracle_pda);
        println!("  rpc_urls       :");
        for (i, url) in self.rpc_urls.iter().enumerate() {
            println!("    [{}] {}", i, url);
        }
        println!("  alert_webhook  : {}", self.alert_webhook.as_deref().unwrap_or("not set"));
        println!("  alert_balance  : {} XNT", self.alert_balance_threshold);
        println!("  dry_run        : {}", self.dry_run);
        println!("  memo_enabled   : {}", self.memo_enabled);
        println!("  tier_threshold : {}ms (cross-tier consensus)", self.tier_consensus_threshold_ms);
        if !self.committee.is_empty() {
            println!("  committee      : {} oracles", self.committee.len());
            for (i, p) in self.committee.iter().enumerate() {
                println!("    [{}] {}", i, p);
            }
        } else {
            println!("  committee      : solo mode");
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config").join("strontium").join("config.json")
}

pub fn status_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config").join("strontium").join("status.json")
}

pub fn default_keypair_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".config").join("strontium").join("oracle-keypair.json")
        .to_string_lossy().to_string()
}

/// Auto-detect vote keypair in common locations
pub fn find_default_vote_keypair() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let candidates = vec![
        Path::new(&home).join(".config").join("solana").join("vote.json"),
        Path::new(&home).join("vote.json"),
    ];
    for path in candidates {
        if path.exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}
