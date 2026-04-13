// N3: Solo mode removed — rotation always active
// When n_oracles=1, window_id % 1 = 0 = always my turn (same behavior as solo mode)

use sha2::{Sha256, Digest};
use std::time::{SystemTime, UNIX_EPOCH};

/// Determine rotation: is it my turn in this window?
/// 
/// Algorithm: primary_index = window_id % n_oracles
/// my_index = position of my pubkey in sorted oracle list
///
/// Staged fallback:
///   t+0s  → primary
///   t+30s → backup-1 if primary silent
///   t+60s → backup-2 if still silent
pub fn rotation_my_turn(
    my_pubkey_bytes: &[u8; 32],
    my_index:        usize,
    n_oracles:       usize,
    interval_s:      u64,
) -> (bool, u64, u64) {
    let now_s = unix_secs();
    let window_id = now_s / interval_s;
    let elapsed   = now_s % interval_s;
    let next_window_s = interval_s - elapsed;

    // N3: n_oracles=1 acts like always-my-turn (replaces solo mode)
    let n = n_oracles.max(1);
    let primary_idx = (window_id % n as u64) as usize;

    // Primary
    if my_index == primary_idx {
        return (true, window_id, next_window_s);
    }

    // Backup-1: submits at +30s if primary silent
    let backup1_idx = (window_id + 1) % n as u64;
    if my_index == backup1_idx as usize && elapsed >= 30 {
        return (true, window_id, next_window_s);
    }

    // Backup-2: submits at +60s if still silent
    let backup2_idx = (window_id + 2) % n as u64;
    if my_index == backup2_idx as usize && elapsed >= 60 {
        return (true, window_id, next_window_s);
    }

    (false, window_id, next_window_s)
}

/// Check if a submission already exists for the current window
pub fn window_has_submission(last_submit_ts: Option<i64>, interval_s: u64) -> bool {
    match last_submit_ts {
        None => false,
        Some(ts) => {
            let now_s   = unix_secs() as i64;
            let ts_s    = ts / 1000;
            let cur_win = now_s as u64 / interval_s;
            let sub_win = ts_s as u64 / interval_s;
            cur_win == sub_win
        }
    }
}

/// State for auto-rotation — discovers active oracles from chain
#[derive(Debug, Default)]
pub struct RotationState {
    pub active_oracles: Vec<[u8; 32]>,
    pub last_refresh:   u64,
}

impl RotationState {
    pub fn new() -> Self {
        Self { active_oracles: Vec::new(), last_refresh: 0 }
    }

    pub fn should_refresh(&self) -> bool {
        unix_secs().saturating_sub(self.last_refresh) > 300
    }

    pub fn my_index(&self, my_pubkey: &[u8; 32]) -> usize {
        self.active_oracles.iter().position(|pk| pk == my_pubkey).unwrap_or(0)
    }

    pub fn n_oracles(&self) -> usize {
        self.active_oracles.len().max(1)
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Slot-hash based election: deterministic, unpredictable
/// Used when slot hash is available from RPC
pub fn slot_hash_election(
    my_pubkey:   &[u8; 32],
    slot_hash:   &[u8; 32],
    window_id:   u64,
    n_oracles:   usize,
    top_n:       usize,
) -> bool {
    let n = n_oracles.max(1);
    let elected_count = top_n.min(n);

    let mut scores: Vec<([u8; 32], [u8; 32])> = Vec::new();

    // For each oracle slot, compute SHA256(slot_hash || window_id || index)
    // In full v2 implementation, oracle list comes from chain
    // For now, use my_pubkey directly
    let mut h = Sha256::new();
    h.update(slot_hash);
    h.update(&window_id.to_le_bytes());
    h.update(my_pubkey);
    let score: [u8; 32] = h.finalize().into();
    scores.push((*my_pubkey, score));

    // If I'm in top elected_count, submit
    // Simplified: if only 1 oracle, always elected
    if n == 1 { return true; }

    // With multiple oracles this would sort all scores and check position
    // For now, use simple modulo rotation
    let my_idx = 0usize;
    let primary = (window_id % n as u64) as usize;
    my_idx == primary
}
