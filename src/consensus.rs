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
pub fn compute_consensus(results: &[NtpResult], tier_threshold_ms: i64) -> Option<ConsensusResult> {
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

    // IQR outlier filter — remove sources >2x IQR from median before spread calc
    let filtered: Vec<i64> = {
        let q1 = timestamps[n / 4];
        let q3 = timestamps[3 * n / 4];
        let iqr = (q3 - q1).max(5); // min 5ms IQR floor
        let lo = median_ms - iqr * 3;
        let hi = median_ms + iqr * 3;
        timestamps.iter().filter(|&&t| t >= lo && t <= hi).copied().collect()
    };
    let filtered = if filtered.len() >= MIN_SOURCES { &filtered } else { &timestamps };

    // Spread (max - min) on filtered set
    let spread_ms = filtered[filtered.len()-1] - filtered[0];

    // Leap second detection: if spread is between 400ms and 1100ms
    // this is likely a leap second event (smearing vs stepping servers)
    // Log warning but still return None — silence is correct behavior
    if (400..=1100).contains(&spread_ms) {
        eprintln!(
            "[warn] ⚠ LEAP SECOND? Sources diverge {}ms — possible leap second event.              Staying silent. Check https://www.ietf.org/timezones/data/leap-seconds.list",
            spread_ms
        );
        return None;
    }

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

    // Cross-tier validation: at least one T-1 or T-2 must agree with median
    // Prevents Pool-only manipulation (Theo's security recommendation)
    let has_quality_source = results.iter().any(|r| {
        matches!(r.tier, NtpTier::Gps | NtpTier::Nts | NtpTier::Stratum1)
        && (r.timestamp_ms - median_ms).abs() <= tier_threshold_ms
    });
    if !has_quality_source {
        // Only Pool sources available or all T-1/T-2 disagree with median
        eprintln!("[warn] No T-1/T-2 source within {}ms of median — staying silent", tier_threshold_ms);
        return None;
    }

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
pub fn run_consensus_cycle(ntp_results: &[NtpResult], tier_threshold_ms: i64) -> Option<ConsensusResult> {
    // If GPS/PPS available, use it as primary source with NTP as cross-check
    if has_gps_pps() {
        if let Some(gps_ms) = get_gps_time_ms() {
            // Cross-check GPS against NTP
            if let Some(ntp_consensus) = compute_consensus(ntp_results, tier_threshold_ms) {
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
    compute_consensus(ntp_results, tier_threshold_ms)
}

/// Determine if it's this operator's turn to submit (window_id rotation)
/// 
/// Algorithm:
///   primary_index = window_id % n_validators
///   my_index = position of my pubkey in sorted committee list
///   → fair round-robin: each operator submits every N windows
///
/// Staged fallback (anti-dublet):
///   t+0s  → primary (primary_index == my_index)
///   t+20s → backup-1 (next in list) if primary silent
///   t+40s → backup-2 if still silent
///
/// Returns (is_my_turn, window_id, seconds_until_next_window)
#[allow(dead_code)]
pub fn rotation_my_turn(
    _my_pubkey_bytes: &[u8; 32],
    my_index: usize,
    n_validators: usize,
    interval_s: u64,
) -> (bool, u64, u64) {
    if n_validators <= 1 {
        return (true, 0, interval_s);
    }

    let now_s               = get_system_clock_ms() / 1000;
    let window_id           = (now_s as u64) / interval_s;
    let window_secs_elapsed = (now_s as u64) % interval_s;
    let window_secs_remain  = interval_s - window_secs_elapsed;

    // Deterministyczna rotacja: window_id % n == mój_indeks
    let primary_index = (window_id % n_validators as u64) as usize;
    let backup1_index = (primary_index + 1) % n_validators;
    let backup2_index = (primary_index + 2) % n_validators;

    // PRIMARY_GRACE_S = czas który dajemy primary na wysłanie TX
    // (NTP query ~3s + TX build + send ~2s + sieć ~1s = ~6s)
    // Backup-1 wchodzi po PRIMARY_GRACE_S od startu okna
    // Backup-2 wchodzi po 2 × PRIMARY_GRACE_S
    const PRIMARY_GRACE_S: u64 = 30;

    let is_my_turn =
        my_index == primary_index
        || (my_index == backup1_index && window_secs_elapsed >= PRIMARY_GRACE_S)
        || (my_index == backup2_index && window_secs_elapsed >= PRIMARY_GRACE_S * 2);

    (is_my_turn, window_id, window_secs_remain)
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
