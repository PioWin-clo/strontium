use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const PROGRAM_ID: &str = "2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe";
pub const ORACLE_PDA: &str = "EtjkQqf1h9gtwHpi2QPRTefWg3XmDfmjQ6YozYZspJzn";

fn config_path() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".config/strontium/config.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrontiumConfig {
    pub keypair_path:            String,
    pub vote_keypair_path:       Option<String>,
    pub interval_s:              u64,
    pub program_id:              String,
    pub oracle_pda:              String,
    pub rpc_urls:                Vec<String>,
    pub alert_webhook:           Option<String>,
    pub alert_balance_threshold: f64,
    pub dry_run:                 bool,
    pub memo_enabled:            bool,
    pub tier_consensus_threshold_ms: i64,
}

impl Default for StrontiumConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/x1pio".to_string());
        Self {
            keypair_path: format!("{}/.config/strontium/oracle-keypair.json", home),
            vote_keypair_path: Some(format!("{}/.config/solana/vote.json", home)),
            interval_s: 300,
            program_id: PROGRAM_ID.to_string(),
            oracle_pda: ORACLE_PDA.to_string(),
            rpc_urls: vec![
                "https://rpc.mainnet.x1.xyz".to_string(),
                "https://api.mainnet.x1.xyz".to_string(),
            ],
            alert_webhook: None,
            alert_balance_threshold: 1.0,
            dry_run: false,
            memo_enabled: true,
            tier_consensus_threshold_ms: 60,
        }
    }
}

impl StrontiumConfig {
    pub fn load() -> Self {
        let path = config_path();
        if let Ok(data) = fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "interval" => {
                self.interval_s = value.parse::<u64>()
                    .map_err(|_| "interval must be a number in seconds".to_string())?;
            }
            "keypair" => self.keypair_path = value.to_string(),
            "vote_keypair" => self.vote_keypair_path = Some(value.to_string()),
            "rpc" => {
                self.rpc_urls.retain(|u| u != value);
                self.rpc_urls.insert(0, value.to_string());
            }
            "dry_run" => self.dry_run = value == "true",
            "memo" => self.memo_enabled = value == "true",
            "tier_threshold" | "tier_consensus_threshold_ms" => {
                self.tier_consensus_threshold_ms = value.parse::<i64>()
                    .map_err(|_| "tier_threshold must be a number in ms".to_string())?;
            }
            "alert_webhook" => self.alert_webhook = Some(value.to_string()),
            "alert_balance" => {
                self.alert_balance_threshold = value.parse::<f64>()
                    .map_err(|_| "alert_balance must be a number".to_string())?;
            }
            _ => return Err(format!("Unknown config key: {}", key)),
        }
        Ok(())
    }

    pub fn display(&self) {
        println!("X1 Strontium — configuration");
        println!("{}", "━".repeat(50));
        println!("  keypair        : {}", self.keypair_path);
        println!("  vote_keypair   : {}", self.vote_keypair_path.as_deref().unwrap_or("auto-detect"));
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
    }
}
