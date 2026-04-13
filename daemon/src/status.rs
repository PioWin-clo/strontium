use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn config_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn status_file() -> PathBuf {
    config_home().join(".config/strontium/status.json")
}

// N5: Priority system — critical reasons cannot be overwritten by lower priority
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SilentReason {
    NoValidSources,
    SpreadTooHigh,
    LowConfidence,
    NotElected,
    RegistrationExpired,
    InsufficientBalance,
    TxRejected,
    NoHealthyRpc,
    DryRun,
    TimestampOutlier,
}

impl SilentReason {
    pub fn priority(&self) -> u8 {
        match self {
            SilentReason::InsufficientBalance => 100,
            SilentReason::RegistrationExpired => 90,
            SilentReason::NoHealthyRpc        => 50,
            SilentReason::TxRejected          => 50,
            SilentReason::TimestampOutlier    => 40,
            SilentReason::NoValidSources      => 30,
            SilentReason::SpreadTooHigh       => 30,
            SilentReason::LowConfidence       => 30,
            SilentReason::NotElected          => 10,
            SilentReason::DryRun              => 5,
        }
    }
}

impl std::fmt::Display for SilentReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SilentReason::NoValidSources      => "no valid NTP sources",
            SilentReason::SpreadTooHigh       => "NTP spread too high",
            SilentReason::LowConfidence       => "confidence too low",
            SilentReason::NotElected          => "not elected (rotation)",
            SilentReason::RegistrationExpired => "registration expired",
            SilentReason::InsufficientBalance => "insufficient balance",
            SilentReason::TxRejected          => "tx rejected by network",
            SilentReason::NoHealthyRpc        => "no healthy RPC",
            SilentReason::DryRun              => "dry-run mode",
            SilentReason::TimestampOutlier    => "timestamp outlier (>5s from system clock)",
        };
        write!(f, "{}", s)
    }
}

// Fix 3: NtpTier defined only here, imported by ntp_client
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NtpTier { Gps, Nts, Stratum1, Pool }

impl std::fmt::Display for NtpTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NtpTier::Gps      => write!(f, "GPS/PPS"),
            NtpTier::Nts      => write!(f, "NTS"),
            NtpTier::Stratum1 => write!(f, "Stratum-1"),
            NtpTier::Pool     => write!(f, "Pool"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NtpSourceStatus {
    pub host:      String,
    pub tier:      NtpTier,
    pub rtt_ms:    i64,
    pub offset_ms: i64,
    pub stratum:   u8,
    pub active:    bool,
}

// N1: Liveness tracking removed — replaced by P11 alert_webhook
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonStatus {
    pub running:             bool,
    pub pid:                 Option<u32>,
    pub oracle_pubkey:       Option<String>,
    pub balance_xnt:         f64,
    pub days_remaining:      f64,
    pub balance_warning:     bool,
    pub last_submit_ts:      Option<i64>,
    pub last_submit_tx:      Option<String>,
    pub last_attempt_ts:     Option<i64>,
    pub last_error:          Option<String>,
    pub silent_cycles:       u32,
    pub silent_reason:       Option<SilentReason>,
    pub interval_s:          u64,
    pub dry_run:             bool,
    pub consensus_ms:        Option<i64>,
    pub spread_ms:           Option<i64>,
    pub confidence:          Option<f64>,
    pub sources_bitmap:      u32,
    pub ntp_sources:         Vec<NtpSourceStatus>,
    pub rotation_window_id:  Option<u64>,
    pub rotation_is_my_turn: Option<bool>,
}

impl DaemonStatus {
    pub fn load() -> Self {
        let path = status_file();
        if let Ok(data) = fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = status_file();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&path, json);
        }
    }

    // N5: Set silent reason respecting priority
    pub fn set_silent_reason(&mut self, reason: SilentReason) {
        let new_prio = reason.priority();
        let cur_prio = self.silent_reason.as_ref().map(|r| r.priority()).unwrap_or(0);
        if new_prio >= cur_prio {
            self.silent_reason = Some(reason);
        }
    }

    pub fn print(&self) {
        println!("X1 Strontium — daemon status");
        println!("{}", "━".repeat(50));
        println!("  Daemon         : {}",
            if self.running { format!("running (PID {})", self.pid.unwrap_or(0)) }
            else { "stopped".to_string() });
        if let Some(pk) = &self.oracle_pubkey {
            println!("  Oracle keypair : {}", pk);
        }
        let warn = if self.balance_warning { " ⚠" } else { " [OK]" };
        println!("  Balance        : {:.3} XNT (~{:.0} days){}",
            self.balance_xnt, self.days_remaining, warn);
        println!("  Last submit    : {}",
            self.last_submit_ts.map(|t| format_ts(t)).unwrap_or_else(|| "never".to_string()));
        println!("  Interval       : {}s", self.interval_s);
        println!("  Mode           : {}", if self.dry_run { "dry-run" } else { "live" });
        if let Some(ms) = self.consensus_ms {
            println!("  NTP consensus  : {}", format_ts(ms));
            if let Some(sp) = self.spread_ms {
                println!("  Spread         : {}ms", sp);
            }
            if let Some(c) = self.confidence {
                println!("  Confidence     : {:.2}", c);
            }
            println!("  Sources active : {}/{}",
                self.ntp_sources.iter().filter(|s| s.active).count(),
                self.ntp_sources.len());
        }
        if self.silent_cycles > 0 {
            println!("  Silent cycles  : {} ({})",
                self.silent_cycles,
                self.silent_reason.as_ref().map(|r| r.to_string()).unwrap_or_default());
        }
    }

    pub fn print_sources(&self) {
        if self.ntp_sources.is_empty() {
            println!("No NTP source data. Is the daemon running?");
            return;
        }
        println!("{:<32} {:>10} {:>6} {:>8} {:>8}",
            "Host", "Tier", "Strat", "RTT", "Offset");
        println!("{}", "─".repeat(70));
        for s in &self.ntp_sources {
            println!("{:<32} {:>10} {:>6} {:>7}ms {:>7}ms",
                s.host, s.tier.to_string(), s.stratum, s.rtt_ms, s.offset_ms);
        }
    }
}

fn format_ts(ms: i64) -> String {
    let s    = ms / 1000;
    let frac = ms % 1000;
    format!("{:02}:{:02}:{:02}.{:03} UTC",
        (s % 86400) / 3600, (s % 3600) / 60, s % 60, frac)
}
