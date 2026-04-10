use crate::ntp_client::{NtpResult, get_gps_time_ms, get_system_clock_ms, has_gps_pps};
use crate::status::NtpTier;

/// Result of a consensus cycle
#[derive(Debug, Clone)]
pub struct ConsensusResult {
    pub timestamp_ms:   i64,
    pub spread_ms:      i64,
    pub confidence:     f64,    // 0.0 - 1.0
    pub sources_used:   u8,
    pub sources_bitmap: u8,     // bitmask of sources that contributed
    pub is_gps:         bool,
    pub sources:        Vec<NtpResult>,
}

/// Thresholds
pub const MAX_SPREAD_MS:       i64  = 50;   // reject consensus if spread > 50ms
pub const MIN_CONFIDENCE:      f64  = 0.60; // don't submit if confidence < 0.60
pub const MIN_SOURCES:         usize = 2;   // minimum sources for consensus
pub const SYSTEM_CLOCK_MAX_DRIFT_MS: i64 = 100; // sanity check: system clock vs NTP

/// Build sources_bitmap from a list of results
/// Bit positions map to NTP_SOURCES array index
pub fn build_sources_bitmap(results: &[NtpResult]) -> u8 {
    use crate::ntp_client::NTP_SOURCES;
    let mut bitmap = 0u8;
    for (i, source) in NTP_SOURCES.iter().enumerate().take(8) {
        if results.iter().any(|r| r.host == source.host) {
            bitmap |= 1 << i;
        }
    }
    bitmap
}

/// Compute NTP consensus from parallel results
/// Returns None if consensus cannot be reached (spread too high, too few sources)
pub fn compute_consensus(results: &[NtpResult]) -> Option<ConsensusResult> {
    if results.len() < MIN_SOURCES { return None; }

    // Sort timestamps
    let mut timestamps: Vec<i64> = results.iter().map(|r| r.timestamp_ms).collect();
    timestamps.sort_unstable();

    let n = timestamps.len();

    // Median
    let median_ms = if n.is_multiple_of(2) {
        let a = timestamps[n/2 - 1];
        let b = timestamps[n/2];
        a.checked_add(b).map(|s| s / 2).unwrap_or(a)
    } else {
        timestamps[n / 2]
    };

    // Spread (max - min)
    let spread_ms = timestamps[n-1] - timestamps[0];
    if spread_ms > MAX_SPREAD_MS { return None; }

    // Sanity check against system clock (NOT a vote, just a check)
    let sysclock_ms = get_system_clock_ms();
    let sysclock_drift = (median_ms - sysclock_ms).abs();
    if sysclock_drift > SYSTEM_CLOCK_MAX_DRIFT_MS * 10 {
        // System clock is extremely off — log but don't block (NTP is truth)
        eprintln!("[warn] System clock drift from NTP: {}ms — check chrony/ntpd", sysclock_drift);
    }

    // Confidence calculation based on:
    // 1. Number of sources (more = higher confidence)
    // 2. Spread (smaller = higher confidence)
    // 3. Tier quality (NTS/stratum1 = higher confidence)
    let source_factor = (results.len() as f64 / 5.0).min(1.0);

    let spread_factor = if spread_ms == 0 {
        1.0
    } else {
        (1.0 - (spread_ms as f64 / MAX_SPREAD_MS as f64)).max(0.0)
    };

    let tier_factor = {
        let nts_count    = results.iter().filter(|r| matches!(r.tier, NtpTier::Gps | NtpTier::Nts)).count();
        let s1_count     = results.iter().filter(|r| matches!(r.tier, NtpTier::Stratum1)).count();
        let quality      = (nts_count as f64 * 1.0 + s1_count as f64 * 0.8) / results.len() as f64;
        quality.min(1.0)
    };

    let confidence = (source_factor * 0.4 + spread_factor * 0.4 + tier_factor * 0.2)
        .clamp(0.0, 1.0);

    if confidence < MIN_CONFIDENCE { return None; }

    let sources_bitmap = build_sources_bitmap(results);
    let is_gps         = results.iter().any(|r| matches!(r.tier, NtpTier::Gps));

    Some(ConsensusResult {
        timestamp_ms: median_ms,
        spread_ms,
        confidence,
        sources_used:   results.len() as u8,
        sources_bitmap,
        is_gps,
        sources:        results.to_vec(),
    })
}

/// Full consensus cycle including GPS/PPS if available
pub fn run_consensus_cycle(ntp_results: &[NtpResult]) -> Option<ConsensusResult> {
    // If GPS/PPS available, use it as primary source with NTP as cross-check
    if has_gps_pps() {
        if let Some(gps_ms) = get_gps_time_ms() {
            // Cross-check GPS against NTP
            if let Some(ntp_consensus) = compute_consensus(ntp_results) {
                let drift = (gps_ms - ntp_consensus.timestamp_ms).abs();
                if drift < 5000 {
                    // GPS and NTP agree — use GPS (more accurate)
                    return Some(ConsensusResult {
                        timestamp_ms:   gps_ms,
                        spread_ms:      drift,
                        confidence:     0.99, // GPS is highest confidence
                        sources_used:   ntp_consensus.sources_used,
                        sources_bitmap: ntp_consensus.sources_bitmap | 0b1000_0000, // GPS bit
                        is_gps:         true,
                        sources:        ntp_consensus.sources,
                    });
                } else {
                    eprintln!("[warn] GPS/PPS drift from NTP: {}ms — GPS may be misconfigured", drift);
                    // Fall through to NTP consensus
                }
            }
        }
    }

    // Standard NTP consensus
    compute_consensus(ntp_results)
}

/// Determine if it's this operator's turn to submit (window_id rotation)
/// Returns (is_my_turn, window_id, seconds_until_next_turn)
pub fn rotation_my_turn(
    my_pubkey_bytes: &[u8; 32],
    n_validators: usize,
    interval_s: u64,
) -> (bool, u64, u64) {
    if n_validators == 0 {
        return (true, 0, interval_s);
    }

    let now_s      = get_system_clock_ms() / 1000;
    let window_id  = (now_s as u64) / interval_s;

    // Deterministic leader: H(window_id || committee_root) % n
    // Simple hash using pubkey bytes + window_id
    let mut hash_input = [0u8; 40];
    hash_input[..32].copy_from_slice(my_pubkey_bytes);
    hash_input[32..].copy_from_slice(&window_id.to_le_bytes());

    // Simple hash (use sha2 for production)
    let hash_val = simple_hash(&hash_input);
    let my_index = hash_val % n_validators as u64;

    // For now with n_validators from config — actual index from sorted validator list
    // In daemon this will be called with actual sorted list position
    let is_primary = my_index == 0;

    let window_secs_elapsed = (now_s as u64) % interval_s;
    let window_secs_remain  = interval_s - window_secs_elapsed;

    // Staged backup: primary at t=0, backup1 at t+20s, backup2 at t+40s
    let is_my_turn = if n_validators == 1 {
        true
    } else {
        is_primary
        || (my_index == 1 && window_secs_elapsed >= 20)
        || (my_index == 2 && window_secs_elapsed >= 40)
    };

    (is_my_turn, window_id, window_secs_remain)
}

/// Simple hash for rotation (not cryptographic — just deterministic)
fn simple_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in data {
        h = h.wrapping_mul(1099511628211);
        h ^= b as u64;
    }
    h
}

/// Check if a submission was already made for this window
/// (anti-dublet: don't submit if fallback already covered this window)
#[allow(dead_code)]
pub fn window_has_submission(last_submit_ts_ms: Option<i64>, interval_s: u64) -> bool {
    match last_submit_ts_ms {
        None => false,
        Some(ts) => {
            let now_s       = get_system_clock_ms() / 1000;
            let last_s      = ts / 1000;
            let current_win = (now_s as u64) / interval_s;
            let last_win    = (last_s as u64) / interval_s;
            current_win == last_win
        }
    }
}
