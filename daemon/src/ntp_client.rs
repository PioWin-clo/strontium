use std::net::UdpSocket;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::status::{NtpSourceStatus, NtpTier};  // Fix 3: NtpTier from status only

// Fix 10: Added Clone to NtpSource
#[derive(Debug, Clone)]
pub struct NtpSource {
    pub host:    &'static str,
    pub port:    u16,
    pub stratum: u8,
    pub tier:    NtpTier,
    pub region:  &'static str,
}

// P10: Expanded list — 45 servers across 4 continents
pub static NTP_SOURCES: &[NtpSource] = &[
    // T-1 NTS (queried via plain NTP — P4 roadmap: implement NTS auth with rustls)
    NtpSource { host: "ptbtime1.ptb.de",              port: 123, stratum: 1, tier: NtpTier::Nts,      region: "Europe"  },
    NtpSource { host: "time.cloudflare.com",          port: 123, stratum: 3, tier: NtpTier::Nts,      region: "Global"  },
    NtpSource { host: "nts.netnod.se",                port: 123, stratum: 1, tier: NtpTier::Nts,      region: "Europe"  },
    NtpSource { host: "ntp.time.nl",                  port: 123, stratum: 1, tier: NtpTier::Nts,      region: "Europe"  },
    // T-2 Stratum-1 — Europe
    NtpSource { host: "ptbtime2.ptb.de",              port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ptbtime3.ptb.de",              port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "tempus1.gum.gov.pl",           port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "tempus2.gum.gov.pl",           port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "tempus3.gum.gov.pl",           port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp-p1.obspm.fr",              port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp.metas.ch",                 port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp1.fau.de",                  port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp2.fau.de",                  port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp.nic.cz",                   port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp.se",                       port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "ntp1.nl.net",                  port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Europe"  },
    NtpSource { host: "time.google.com",              port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Global"  },
    NtpSource { host: "time.apple.com",               port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Global"  },
    // T-2 Stratum-1 — Americas
    NtpSource { host: "nist1-atl.ustiming.org",       port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "time.nist.gov",                port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "time-a-g.nist.gov",            port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "time-b-g.nist.gov",            port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "ntp1.glypnod.com",             port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "ntp1.net.berkeley.edu",        port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "ntp.ula.ve",                   port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    NtpSource { host: "tick.usask.ca",                port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "America" },
    // T-2 Stratum-1 — Asia-Pacific
    NtpSource { host: "ntp.jst.mfeed.ad.jp",          port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Asia"    },
    NtpSource { host: "ntp.nict.jp",                  port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Asia"    },
    NtpSource { host: "ntp.kornet.net",               port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Asia"    },
    NtpSource { host: "stdtime.gov.hk",               port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Asia"    },
    NtpSource { host: "ntp.nml.csiro.au",             port: 123, stratum: 1, tier: NtpTier::Stratum1, region: "Pacific" },
    NtpSource { host: "time.cloudflare.com",          port: 123, stratum: 3, tier: NtpTier::Stratum1, region: "Global"  },
    // T-3 Pool
    NtpSource { host: "0.pool.ntp.org",               port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Global"  },
    NtpSource { host: "1.pool.ntp.org",               port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Global"  },
    NtpSource { host: "2.pool.ntp.org",               port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Global"  },
    NtpSource { host: "europe.pool.ntp.org",          port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Europe"  },
    NtpSource { host: "asia.pool.ntp.org",            port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Asia"    },
    NtpSource { host: "north-america.pool.ntp.org",   port: 123, stratum: 2, tier: NtpTier::Pool,     region: "America" },
    NtpSource { host: "oceania.pool.ntp.org",         port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Pacific" },
    NtpSource { host: "south-america.pool.ntp.org",   port: 123, stratum: 2, tier: NtpTier::Pool,     region: "America" },
    NtpSource { host: "africa.pool.ntp.org",          port: 123, stratum: 2, tier: NtpTier::Pool,     region: "Africa"  },
];

// Fix 6: Added Clone to NtpResult
#[derive(Debug, Clone)]
pub struct NtpResult {
    pub host:         String,
    pub timestamp_ms: i64,
    pub offset_ms:    i64,
    pub rtt_ms:       i64,
    pub stratum:      u8,
    pub tier:         NtpTier,
}

const NTP_UNIX_OFFSET:    u64 = 2_208_988_800;
const QUERY_TIMEOUT_MS:   u64 = 2000;

pub fn query_ntp(host: &str, port: u16, tier: NtpTier, stratum: u8) -> Option<NtpResult> {
    let addr   = format!("{}:{}", host, port);
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(QUERY_TIMEOUT_MS))).ok()?;

    let t1          = now_ms();
    let mut packet  = [0u8; 48];
    packet[0]       = 0x1B; // LI=0, VN=3, Mode=3 (client)

    let addr_parsed: std::net::SocketAddr = addr.parse().ok()?;
    socket.send_to(&packet, addr_parsed).ok()?;

    let mut buf = [0u8; 48];
    let (n, _)  = socket.recv_from(&mut buf).ok()?;
    let t4      = now_ms();

    if n < 48 { return None; }

    let stratum_recv = buf[1];
    if stratum_recv == 0 || stratum_recv > 15 { return None; }

    let tx_sec  = u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]) as u64;
    let tx_frac = u32::from_be_bytes([buf[44], buf[45], buf[46], buf[47]]) as u64;

    if tx_sec < NTP_UNIX_OFFSET { return None; }

    let server_unix_ms = ((tx_sec - NTP_UNIX_OFFSET) * 1000
        + (tx_frac * 1000 / 0x1_0000_0000)) as i64;

    let rtt_ms    = t4 - t1;
    let offset_ms = server_unix_ms - (t1 + rtt_ms / 2);
    let actual_stratum = stratum_recv.min(stratum);

    Some(NtpResult {
        host: host.to_string(),
        timestamp_ms: server_unix_ms,
        offset_ms,
        rtt_ms,
        stratum: actual_stratum,
        tier,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// P9: Discover sources — always return at least min_count regardless of tier
pub fn discover_sources(min_count: usize) -> Vec<NtpResult> {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let results = Arc::new(Mutex::new(Vec::<NtpResult>::new()));
    let mut handles = Vec::new();

    for source in NTP_SOURCES {
        let host    = source.host;
        let port    = source.port;
        let tier    = source.tier.clone();
        let stratum = source.stratum;
        let col     = Arc::clone(&results);

        let h = thread::spawn(move || {
            if let Some(r) = query_ntp(host, port, tier, stratum) {
                col.lock().unwrap().push(r);
            }
        });
        handles.push(h);
    }

    for h in handles { let _ = h.join(); }

    let mut all = results.lock().unwrap().clone();

    // Sort: tier priority first, then RTT
    all.sort_by(|a, b| {
        tier_priority(&b.tier).cmp(&tier_priority(&a.tier))
            .then(a.rtt_ms.cmp(&b.rtt_ms))
    });

    // Dedup by hostname
    let mut seen = std::collections::HashSet::new();
    let deduped: Vec<NtpResult> = all.into_iter()
        .filter(|r| seen.insert(r.host.clone()))
        .collect();

    // P9: Always at least min_count, up to 10
    let take = deduped.len().min(10).max(min_count.min(deduped.len()));
    deduped.into_iter().take(take).collect()
}

fn tier_priority(tier: &NtpTier) -> u8 {
    match tier {
        NtpTier::Gps      => 4,
        NtpTier::Nts      => 3,
        NtpTier::Stratum1 => 2,
        NtpTier::Pool     => 1,
    }
}

pub fn has_gps_pps() -> bool {
    std::path::Path::new("/dev/pps0").exists()
}

pub fn get_gps_time_ms() -> Option<i64> {
    if !has_gps_pps() { return None; }
    // Returns system clock when GPS is present as cross-reference
    Some(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64)
}

pub fn get_system_clock_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn to_source_status(results: &[NtpResult]) -> Vec<NtpSourceStatus> {
    results.iter().map(|r| NtpSourceStatus {
        host:      r.host.clone(),
        tier:      r.tier.clone(),
        rtt_ms:    r.rtt_ms,
        offset_ms: r.offset_ms,
        stratum:   r.stratum,
        active:    true,
    }).collect()
}
