use crate::ntp_client::{NtpResult, get_gps_time_ms, get_system_clock_ms, has_gps_pps};
use crate::status::NtpTier;

#[derive(Debug, Clone)]
pub struct ConsensusResult {
    pub timestamp_ms:   i64,
    pub spread_ms:      i64,
    pub confidence:     f64,
    pub sources_used:   u8,
    pub sources_bitmap: u32,  // P5: u32
    pub is_gps:         bool,
    pub sources:        Vec<NtpResult>,
}

pub const MAX_SPREAD_MS:            i64   = 50;
pub const MIN_CONFIDENCE:           f64   = 0.60;
pub const MIN_SOURCES:              usize = 2;
pub const SYSTEM_CLOCK_MAX_DRIFT_MS: i64  = 5_000; // 5 seconds vs system clock

pub fn build_sources_bitmap(results: &[NtpResult]) -> u32 {
    use crate::ntp_client::NTP_SOURCES;
    let mut bitmap = 0u32;
    for (i, source) in NTP_SOURCES.iter().enumerate().take(32) {
        if results.iter().any(|r| r.host == source.host) {
            bitmap |= 1 << i;
        }
    }
    bitmap
}

pub fn compute_consensus(results: &[NtpResult], tier_threshold_ms: i64) -> Option<ConsensusResult> {
    if results.len() < MIN_SOURCES { return None; }

    let mut timestamps: Vec<i64> = results.iter().map(|r| r.timestamp_ms).collect();
    timestamps.sort_unstable();
    let n = timestamps.len();

    let median_ms = if n % 2 == 0 {
        let a = timestamps[n/2 - 1];
        let b = timestamps[n/2];
        a.checked_add(b).map(|s| s / 2).unwrap_or(a)
    } else {
        timestamps[n / 2]
    };

    // IQR outlier filter — remove sources >3×IQR from median before spread calc
    let filtered: Vec<i64> = {
        let q1 = timestamps[n / 4];
        let q3 = timestamps[3 * n / 4];
        let iqr = (q3 - q1).max(5);
        let lo = median_ms - iqr * 3;
        let hi = median_ms + iqr * 3;
        timestamps.iter().filter(|&&t| t >= lo && t <= hi).copied().collect()
    };
    let filtered = if filtered.len() >= MIN_SOURCES { &filtered } else { &timestamps };

    // Leap second detection
    let raw_spread = filtered[filtered.len()-1] - filtered[0];
    if (400..=1100).contains(&raw_spread) {
        eprintln!("[warn] ⚠ Possible leap second event ({}ms spread) — staying silent", raw_spread);
        return None;
    }

    let spread_ms = raw_spread;
    if spread_ms > MAX_SPREAD_MS { return None; }

    // Confidence calculation
    let source_factor = (results.len() as f64 / 5.0).min(1.0);
    let spread_factor = if spread_ms == 0 { 1.0 }
        else { (1.0 - (spread_ms as f64 / MAX_SPREAD_MS as f64)).max(0.0) };
    let tier_factor = {
        let nts = results.iter().filter(|r| matches!(r.tier, NtpTier::Gps | NtpTier::Nts)).count();
        let s1  = results.iter().filter(|r| matches!(r.tier, NtpTier::Stratum1)).count();
        ((nts as f64 * 1.0 + s1 as f64 * 0.8) / results.len() as f64).min(1.0)
    };
    let confidence = (source_factor * 0.4 + spread_factor * 0.4 + tier_factor * 0.2).clamp(0.0, 1.0);
    if confidence < MIN_CONFIDENCE { return None; }

    // N2: GPS is exempt from cross-tier validation
    let is_gps = results.iter().any(|r| matches!(r.tier, NtpTier::Gps));
    if !is_gps {
        // Cross-tier validation: at least one T-1/T-2 must agree with median
        let has_quality = results.iter().any(|r| {
            matches!(r.tier, NtpTier::Gps | NtpTier::Nts | NtpTier::Stratum1)
            && (r.timestamp_ms - median_ms).abs() <= tier_threshold_ms
        });
        if !has_quality {
            eprintln!("[warn] No T-1/T-2 source within {}ms of median — staying silent", tier_threshold_ms);
            return None;
        }
    }

    let sources_bitmap = build_sources_bitmap(results);

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

pub fn run_consensus_cycle(ntp_results: &[NtpResult], tier_threshold_ms: i64) -> Option<ConsensusResult> {
    // N2: GPS as primary with NTP cross-check
    if has_gps_pps() {
        if let Some(gps_ms) = get_gps_time_ms() {
            let sys_ms = get_system_clock_ms();
            let sys_drift = (gps_ms - sys_ms).abs();
            if sys_drift < 5_000 {
                // GPS and system clock agree — use GPS, skip cross-tier
                if let Some(ntp) = compute_consensus(ntp_results, tier_threshold_ms) {
                    let gps_ntp_drift = (gps_ms - ntp.timestamp_ms).abs();
                    if gps_ntp_drift < 5_000 {
                        return Some(ConsensusResult {
                            timestamp_ms:   gps_ms,
                            spread_ms:      gps_ntp_drift,
                            confidence:     0.99,
                            sources_used:   ntp.sources_used,
                            sources_bitmap: ntp.sources_bitmap | 0x8000_0000,
                            is_gps:         true,
                            sources:        ntp.sources,
                        });
                    }
                }
                // GPS alone (no NTP) — still valid, N2: exempt from cross-tier
                return Some(ConsensusResult {
                    timestamp_ms:   gps_ms,
                    spread_ms:      0,
                    confidence:     0.99,
                    sources_used:   1,
                    sources_bitmap: 0x8000_0000,
                    is_gps:         true,
                    sources:        vec![],
                });
            } else {
                eprintln!("[warn] GPS drift from system clock: {}ms — GPS may be misconfigured", sys_drift);
            }
        }
    }

    compute_consensus(ntp_results, tier_threshold_ms)
}
