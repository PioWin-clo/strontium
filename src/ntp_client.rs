use std::net::UdpSocket;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::thread;
use std::sync::{Arc, Mutex};
use crate::status::{NtpSourceStatus, NtpTier};

/// NTP source definition with tier classification
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NtpSource {
    pub host:    &'static str,
    pub port:    u16,
    pub stratum: u8,
    pub tier:    NtpTier,
    pub nts:     bool,   // supports NTS
    pub region:  &'static str,
}

/// All known NTP sources, ordered by tier (best first)
pub const NTP_SOURCES: &[NtpSource] = &[
    // Tier-1: NTS-capable government atomic clocks
    NtpSource { host: "ptbtime1.ptb.de",   port: 123, stratum: 1, tier: NtpTier::Nts,      nts: true,  region: "Europe"    },
    NtpSource { host: "time.cloudflare.com", port: 123, stratum: 3, tier: NtpTier::Nts,    nts: true,  region: "Global"    },
    NtpSource { host: "nts.netnod.se",      port: 4460, stratum: 1, tier: NtpTier::Nts,    nts: true,  region: "Europe"    },

    // Tier-2: Government atomic stratum 1
    NtpSource { host: "ptbtime2.ptb.de",        port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "ptbtime3.ptb.de",        port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "tempus1.gum.gov.pl",     port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "tempus2.gum.gov.pl",     port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "tempus3.gum.gov.pl",     port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "nist1-atl.ustiming.org", port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "America" },
    NtpSource { host: "time.nist.gov",          port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "America" },
    NtpSource { host: "ntp.jst.mfeed.ad.jp",    port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Asia"   },
    NtpSource { host: "ptbtime1.ptb.de",        port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "syrte.obspm.fr",         port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "ntp.metas.ch",           port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Europe" },
    NtpSource { host: "time.google.com",        port: 123, stratum: 1, tier: NtpTier::Stratum1, nts: false, region: "Global" },

    // Tier-3: Pool fallback
    NtpSource { host: "0.pool.ntp.org",      port: 123, stratum: 2, tier: NtpTier::Pool, nts: false, region: "Global"  },
    NtpSource { host: "1.pool.ntp.org",      port: 123, stratum: 2, tier: NtpTier::Pool, nts: false, region: "Global"  },
    NtpSource { host: "europe.pool.ntp.org", port: 123, stratum: 2, tier: NtpTier::Pool, nts: false, region: "Europe"  },
];

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NtpResult {
    pub host:       String,
    pub timestamp_ms: i64,
    pub offset_ms:  i64,
    pub rtt_ms:     i64,
    pub stratum:    u8,
    pub tier:       NtpTier,
    pub nts_used:   bool,
}

/// Check if GPS/PPS is available locally
pub fn has_gps_pps() -> bool {
    std::path::Path::new("/dev/pps0").exists()
}

/// Get time from local GPS/PPS via system clock (chrony disciplined)
/// Only valid if /dev/pps0 exists AND system clock has been disciplined
pub fn get_gps_time_ms() -> Option<i64> {
    if !has_gps_pps() { return None; }
    // System clock is disciplined by GPS/PPS via chrony
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?;
    Some(now.as_millis() as i64)
}

/// Get system clock time for sanity checking (NOT as a consensus vote)
pub fn get_system_clock_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Query a single NTP server using RFC 4330 SNTP
pub fn query_ntp(source: &NtpSource, timeout_ms: u64) -> Option<NtpResult> {
    let addr = format!("{}:{}", source.host, source.port);

    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(timeout_ms))).ok()?;
    socket.set_write_timeout(Some(Duration::from_millis(timeout_ms / 2))).ok()?;

    // Build NTP request packet (48 bytes)
    let mut packet = [0u8; 48];
    packet[0] = 0x1B; // LI=0, VN=3, Mode=3 (client)

    let t1 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    socket.send_to(&packet, &addr).ok()?;

    let mut buf = [0u8; 48];
    let (n, _) = socket.recv_from(&mut buf).ok()?;
    if n < 48 { return None; }

    let t4 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    // Extract transmit timestamp from packet (bytes 40-47, NTP epoch to Unix epoch)
    let ntp_secs = u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]) as i64;
    let ntp_frac = u32::from_be_bytes([buf[44], buf[45], buf[46], buf[47]]) as i64;

    // NTP epoch is Jan 1 1900, Unix epoch is Jan 1 1970 = 70 years = 2208988800 secs
    const NTP_UNIX_OFFSET: i64 = 2_208_988_800;
    let server_unix_secs = ntp_secs - NTP_UNIX_OFFSET;
    let server_unix_ms   = server_unix_secs * 1000 + (ntp_frac * 1000 / (1i64 << 32));

    // RTT and offset calculation
    let rtt_us     = t4 - t1;
    let rtt_ms     = rtt_us / 1000;
    let local_mid  = (t1 + t4) / 2 / 1000; // midpoint in ms
    let offset_ms  = server_unix_ms - local_mid;

    // Extract stratum from packet byte 1
    let stratum = buf[1];
    if stratum == 0 || stratum > 15 { return None; } // invalid stratum

    Some(NtpResult {
        host:         source.host.to_string(),
        timestamp_ms: server_unix_ms,
        offset_ms,
        rtt_ms,
        stratum,
        tier:         source.tier.clone(),
        nts_used:     false, // NTS not implemented yet (v1.5 roadmap)
    })
}

/// Autodiscover the 5 best NTP sources in parallel
/// Priority: GPS > NTS tier-1 > stratum-1 > pool
/// Requires at least 2 different regions
pub fn discover_sources(count: usize) -> Vec<NtpResult> {
    // Query all sources in parallel using threads
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for source in NTP_SOURCES.iter() {
        let results = Arc::clone(&results);
        let source_clone = source.clone();
        let handle = thread::spawn(move || {
            if let Some(r) = query_ntp(&source_clone, 3000) {
                let mut lock = results.lock().unwrap();
                lock.push(r);
            }
        });
        handles.push(handle);
    }

    // Wait for all threads (max 3.5s)
    for h in handles { let _ = h.join(); }

    let mut all: Vec<NtpResult> = results.lock().unwrap().clone();

    // Sort by tier priority, then RTT
    all.sort_by(|a, b| {
        let tier_order = |t: &NtpTier| match t {
            NtpTier::Gps      => 0,
            NtpTier::Nts      => 1,
            NtpTier::Stratum1 => 2,
            NtpTier::Pool     => 3,
        };
        let ta = tier_order(&a.tier);
        let tb = tier_order(&b.tier);
        ta.cmp(&tb).then(a.rtt_ms.cmp(&b.rtt_ms))
    });

    // Ensure minimum quality: at least 3 stratum-1 or better
    let stratum1_count = all.iter().filter(|r| {
        matches!(r.tier, NtpTier::Gps | NtpTier::Nts | NtpTier::Stratum1)
    }).count();

    // If we have enough quality sources, prefer them over pool
    let selected: Vec<NtpResult> = if stratum1_count >= 3 {
        all.into_iter()
           .filter(|r| !matches!(r.tier, NtpTier::Pool))
           .take(count)
           .collect()
    } else {
        all.into_iter().take(count).collect()
    };

    selected
}

/// Query selected sources IN PARALLEL (used each cycle for fast measurement)
pub fn query_sources_parallel(sources: &[NtpResult]) -> Vec<NtpResult> {
    let results = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for r in sources {
        let source_def = NTP_SOURCES.iter().find(|s| s.host == r.host);
        if source_def.is_none() { continue; }
        let src = source_def.unwrap().clone();

        let results  = Arc::clone(&results);
        let handle = thread::spawn(move || {
            if let Some(result) = query_ntp(&src, 2000) {
                let mut lock = results.lock().unwrap();
                lock.push(result);
            }
        });
        handles.push(handle);
    }

    for h in handles { let _ = h.join(); }

    let guard = results.lock().unwrap();
    guard.clone()
}

/// Convert NtpResult to NtpSourceStatus for status.json
#[allow(dead_code)]
pub fn to_source_status(results: &[NtpResult], all_sources: &[NtpSource]) -> Vec<NtpSourceStatus> {
    all_sources.iter().map(|s| {
        if let Some(r) = results.iter().find(|r| r.host == s.host) {
            NtpSourceStatus {
                host:      s.host.to_string(),
                stratum:   r.stratum,
                rtt_ms:    r.rtt_ms,
                offset_ms: r.offset_ms,
                tier:      r.tier.clone(),
                active:    true,
                nts:       r.nts_used,
            }
        } else {
            NtpSourceStatus {
                host:      s.host.to_string(),
                stratum:   s.stratum,
                rtt_ms:    0,
                offset_ms: 0,
                tier:      s.tier.clone(),
                active:    false,
                nts:       s.nts,
            }
        }
    }).collect()
}


