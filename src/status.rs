use std::fs;
use serde::{Deserialize, Serialize};
use crate::config::status_path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonStatus {
    pub running:           bool,
    pub pid:               Option<u32>,
    pub last_submit_ts:    Option<i64>,    // unix ms
    pub last_submit_tx:    Option<String>, // tx signature
    pub last_attempt_ts:   Option<i64>,
    pub silent_cycles:     u32,
    pub silent_reason:     Option<SilentReason>,
    pub balance_xnt:       f64,
    pub days_remaining:    f64,
    pub balance_warning:   bool,
    pub interval_s:        u64,
    pub dry_run:           bool,
    pub ntp_sources:       Vec<NtpSourceStatus>,
    pub consensus_ms:      Option<i64>,
    pub spread_ms:         Option<i64>,
    pub confidence:        Option<f64>,    // 0.0 - 1.0
    pub sources_bitmap:    u8,
    pub rotation_window_id: Option<u64>,
    pub rotation_is_my_turn: Option<bool>,
    pub rotation_next_turn_secs: Option<u64>,
    pub oracle_pubkey:     Option<String>,
    pub last_error:        Option<String>,
    pub submissions_total:    u64,      // wszystkie udane submisje
    pub expected_turns_24h:   u64,      // ile razy powinienem był submitować (24h)
    pub successful_turns_24h: u64,      // ile razy faktycznie submitowałem w swoim oknie
    pub liveness_warning:     bool,     // true gdy activity ratio < 70%
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SilentReason {
    NotElected,
    InsufficientBalance,
    NoHealthyRpc,
    NoValidSources,
    TxRejected,
    DryRun,
    LowConfidence,
    SpreadTooHigh,
    ValidatorNotActive,
    RegistrationExpired,
    TimestampOutlier,
    LeapSecondDetected,
}

impl std::fmt::Display for SilentReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SilentReason::NotElected          => write!(f, "not elected (rotation)"),
            SilentReason::InsufficientBalance => write!(f, "insufficient balance"),
            SilentReason::NoHealthyRpc        => write!(f, "no healthy RPC"),
            SilentReason::NoValidSources      => write!(f, "no valid NTP sources"),
            SilentReason::TxRejected          => write!(f, "tx rejected by network"),
            SilentReason::DryRun              => write!(f, "dry-run mode"),
            SilentReason::LowConfidence       => write!(f, "low confidence"),
            SilentReason::SpreadTooHigh       => write!(f, "NTP spread too high"),
            SilentReason::ValidatorNotActive  => write!(f, "validator not active"),
            SilentReason::RegistrationExpired => write!(f, "registration expired"),
            SilentReason::TimestampOutlier    => write!(f, "timestamp outlier (>10s from chain clock)"),
            SilentReason::LeapSecondDetected  => write!(f, "leap second detected — sources diverge >400ms"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NtpSourceStatus {
    pub host:       String,
    pub stratum:    u8,
    pub rtt_ms:     i64,
    pub offset_ms:  i64,
    pub tier:       NtpTier,
    pub active:     bool,
    pub nts:        bool,     // NTS authenticated
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NtpTier {
    Gps,      // tier-0: GPS/PPS
    Nts,      // tier-1: NTS authenticated
    Stratum1, // tier-2: government atomic
    Pool,     // tier-3: NTP pool fallback
}

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

impl DaemonStatus {
    pub fn load() -> Self {
        let path = status_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = status_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&path, json);
        }
    }

    /// Print human-readable status to stdout
    pub fn print(&self) {
        println!("X1 Strontium — daemon status");
        println!("{}", "━".repeat(50));

        let status_icon = if self.running { "running" } else { "stopped" };
        let pid_str = self.pid.map(|p| format!(" (PID {})", p)).unwrap_or_default();
        println!("  Daemon         : {}{}", status_icon, pid_str);

        if let Some(pk) = &self.oracle_pubkey {
            println!("  Oracle keypair : {}", pk);
        }

        // Balance
        let bal_color = if self.balance_xnt < 0.5 { "CRITICAL" }
                        else if self.balance_xnt < 1.0 { "LOW" }
                        else { "OK" };
        println!("  Balance        : {:.3} XNT (~{:.0} days) [{}]",
            self.balance_xnt, self.days_remaining, bal_color);

        // Last submit
        if let Some(ts) = self.last_submit_ts {
            let secs_ago = (chrono_now_ms() - ts) / 1000;
            let tx = self.last_submit_tx.as_deref().unwrap_or("unknown");
            println!("  Last submit    : {}s ago ({})", secs_ago, shorten(tx, 12));
        } else {
            println!("  Last submit    : never");
        }

        println!("  Interval       : {}s", self.interval_s);
        println!("  Mode           : {}", if self.dry_run { "dry-run" } else { "live" });

        println!();

        // Consensus
        if let Some(ms) = self.consensus_ms {
            println!("  NTP consensus  : {} UTC", format_ts(ms));
        }
        if let Some(spread) = self.spread_ms {
            println!("  Spread         : {}ms", spread);
        }
        if let Some(conf) = self.confidence {
            println!("  Confidence     : {:.2}", conf);
        }
        let n_active = self.ntp_sources.iter().filter(|s| s.active).count();
        let n_total  = self.ntp_sources.len();
        println!("  Sources active : {}/{}", n_active, n_total);

        if self.silent_cycles > 0 {
            let reason = self.silent_reason.as_ref()
                .map(|r| r.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("  Silent cycles  : {} ({})", self.silent_cycles, reason);
        } else {
            println!("  Silent cycles  : 0");
        }

        println!();

        // Rotation
        if let Some(wid) = self.rotation_window_id {
            let my_turn = self.rotation_is_my_turn.unwrap_or(false);
            let next = self.rotation_next_turn_secs
                .map(|s| format!("~{}s", s))
                .unwrap_or_else(|| "unknown".to_string());
            println!("  Rotation       : window {} | my turn: {} | next: {}",
                wid, if my_turn { "YES" } else { "no" }, next);
        }

        if let Some(err) = &self.last_error {
            println!();
            println!("  Last error     : {}", err);
        }
    }

    /// Print NTP sources table
    pub fn print_sources(&self) {
        println!("X1 Strontium — NTP sources");
        println!("{}", "━".repeat(70));
        println!("  {:<30} {:>8} {:>8} {:>8} {:>10} {:>5}",
            "Source", "Stratum", "Tier", "RTT", "Offset", "NTS");
        println!("  {}", "─".repeat(65));
        for s in &self.ntp_sources {
            let icon = if s.active { "✓" } else { "✗" };
            println!("  {} {:<28} {:>7}  {:>8} {:>6}ms {:>+8}ms {:>5}",
                icon, s.host, s.stratum,
                s.tier.to_string(),
                s.rtt_ms, s.offset_ms,
                if s.nts { "yes" } else { "no" });
        }
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn format_ts(ms: i64) -> String {
    let secs = ms / 1000;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn shorten(s: &str, n: usize) -> String {
    if s.len() <= n * 2 + 3 { return s.to_string(); }
    format!("{}...{}", &s[..n], &s[s.len()-n..])
}
