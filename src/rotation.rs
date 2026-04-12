use std::collections::HashSet;

/// Oracle rotation state — auto-discovered from chain, slot-hash based
pub struct RotationState {
    /// Sorted list of active oracle pubkeys (32-byte arrays)
    pub active_oracles: Vec<[u8; 32]>,
    /// Slot at which we last refreshed the list
    pub last_fetch_slot: u64,
    /// Minimum oracle count to enable rotation (solo mode below this)
    pub min_for_rotation: usize,
}

impl RotationState {
    pub fn new() -> Self {
        Self {
            active_oracles:   Vec::new(),
            last_fetch_slot:  0,
            min_for_rotation: 2,
        }
    }

    /// Load oracle list from OracleState.submissions[] via RPC.
    /// Filters out stale entries (not seen within staleness_slots).
    /// Returns true if list changed.
    pub fn refresh_from_submissions(
        &mut self,
        submissions:      &[([u8; 32], u64)], // (pubkey, last_slot)
        current_slot:     u64,
        staleness_slots:  u64,
    ) -> bool {
        let mut fresh: Vec<[u8; 32]> = submissions
            .iter()
            .filter(|(_, slot)| current_slot.saturating_sub(*slot) <= staleness_slots)
            .map(|(pk, _)| *pk)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Deterministic sort — same order on every daemon
        fresh.sort();

        let changed = fresh != self.active_oracles;
        self.active_oracles  = fresh;
        self.last_fetch_slot = current_slot;
        changed
    }

    /// Is rotation enabled? (enough active oracles)
    pub fn rotation_active(&self) -> bool {
        self.active_oracles.len() >= self.min_for_rotation
    }

    /// Compute expected primary oracle for a given window.
    /// Uses SHA256(slot_hash || window_id) % n — unpredictable,
    /// cannot be gamed without controlling the block hash.
    pub fn expected_primary(
        &self,
        slot_hash: &[u8; 32],
        window_id: u64,
    ) -> Option<[u8; 32]> {
        let n = self.active_oracles.len();
        if n == 0 { return None; }

        let hash = rotation_hash(slot_hash, window_id);
        let mut index_bytes = [0u8; 8];
        index_bytes.copy_from_slice(&hash[..8]);
        let idx = u64::from_le_bytes(index_bytes) as usize % n;

        Some(self.active_oracles[idx])
    }

    /// Compute backup oracle at offset N from primary.
    /// offset=1 → backup-1 (enters at t+30s)
    /// offset=2 → backup-2 (enters at t+60s)
    pub fn expected_backup(
        &self,
        slot_hash: &[u8; 32],
        window_id: u64,
        offset:    usize,
    ) -> Option<[u8; 32]> {
        let n = self.active_oracles.len();
        if n < 2 { return None; }

        let hash = rotation_hash(slot_hash, window_id);
        let mut index_bytes = [0u8; 8];
        index_bytes.copy_from_slice(&hash[..8]);
        let primary_idx = u64::from_le_bytes(index_bytes) as usize % n;
        let backup_idx  = (primary_idx + offset) % n;

        Some(self.active_oracles[backup_idx])
    }

    /// Core decision: should THIS oracle submit right now?
    ///
    /// elapsed_secs  = seconds since start of current window
    /// primary_grace = seconds primary has exclusive right (default: 30)
    /// backup_grace  = seconds each backup waits after previous (default: 30)
    pub fn should_submit(
        &self,
        my_pubkey:     &[u8; 32],
        slot_hash:     &[u8; 32],
        window_id:     u64,
        elapsed_secs:  u64,
        primary_grace: u64,
        backup_grace:  u64,
    ) -> SubmitDecision {
        // Solo mode — always submit
        if !self.rotation_active() {
            return SubmitDecision {
                should_submit: true,
                role:          OracleRole::Solo,
                reason:        format!(
                    "rotation disabled: {} active oracle(s), min {} required",
                    self.active_oracles.len(), self.min_for_rotation
                ),
            };
        }

        let primary = match self.expected_primary(slot_hash, window_id) {
            Some(p) => p,
            None    => return SubmitDecision {
                should_submit: true,
                role:          OracleRole::Solo,
                reason:        "rotation: empty oracle list, solo mode".to_string(),
            },
        };

        // Am I primary?
        if *my_pubkey == primary {
            return SubmitDecision {
                should_submit: true,
                role:          OracleRole::Primary,
                reason:        format!("primary for window {}", window_id),
            };
        }

        // Am I backup-1?
        if let Some(backup1) = self.expected_backup(slot_hash, window_id, 1) {
            if *my_pubkey == backup1 && elapsed_secs >= primary_grace {
                return SubmitDecision {
                    should_submit: true,
                    role:          OracleRole::Backup(1),
                    reason:        format!(
                        "backup-1 fallback (primary silent for {}s)", elapsed_secs
                    ),
                };
            }
        }

        // Am I backup-2?
        if let Some(backup2) = self.expected_backup(slot_hash, window_id, 2) {
            if *my_pubkey == backup2 && elapsed_secs >= primary_grace + backup_grace {
                return SubmitDecision {
                    should_submit: true,
                    role:          OracleRole::Backup(2),
                    reason:        format!(
                        "backup-2 fallback (primary+backup-1 silent for {}s)", elapsed_secs
                    ),
                };
            }
        }

        // Not my turn — calculate wait time
        let wait_secs = if self.expected_backup(slot_hash, window_id, 1)
            .map(|b| b == *my_pubkey).unwrap_or(false)
        {
            primary_grace.saturating_sub(elapsed_secs)
        } else {
            (primary_grace + backup_grace).saturating_sub(elapsed_secs)
        };

        SubmitDecision {
            should_submit: false,
            role:          OracleRole::Waiting,
            reason:        format!(
                "not_elected (window {}, primary={}, wait ~{}s)",
                window_id, pubkey_short(&primary), wait_secs
            ),
        }
    }

    /// Human-readable status lines for x1sr status output
    pub fn status_lines(
        &self,
        my_pubkey:     &[u8; 32],
        slot_hash:     &[u8; 32],
        window_id:     u64,
        elapsed_secs:  u64,
        primary_grace: u64,
        backup_grace:  u64,
    ) -> Vec<String> {
        if !self.rotation_active() {
            return vec![
                format!("Rotation  : solo mode ({} active oracle, min {} required)",
                    self.active_oracles.len(), self.min_for_rotation),
            ];
        }

        let primary  = self.expected_primary(slot_hash, window_id).unwrap_or([0u8; 32]);
        let decision = self.should_submit(
            my_pubkey, slot_hash, window_id,
            elapsed_secs, primary_grace, backup_grace,
        );

        vec![
            format!("Rotation  : auto ({} active oracles, slot-hash based)",
                self.active_oracles.len()),
            format!("Window    : {} ({}s elapsed / {}s total)",
                window_id, elapsed_secs, primary_grace + backup_grace * 2),
            format!("Primary   : {}", pubkey_short(&primary)),
            format!("My role   : {} — {}",
                decision.role.label(),
                if decision.should_submit { "SUBMIT ✓" } else { "waiting" }),
            format!("Reason    : {}", decision.reason),
        ]
    }
}

// ─── Types ────────────────────────────────────────────────────────────────────

pub struct SubmitDecision {
    pub should_submit: bool,
    pub role:          OracleRole,
    pub reason:        String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OracleRole {
    Solo,
    Primary,
    Backup(u8),
    Waiting,
}

impl OracleRole {
    pub fn label(&self) -> &str {
        match self {
            OracleRole::Solo      => "solo",
            OracleRole::Primary   => "primary",
            OracleRole::Backup(1) => "backup-1",
            OracleRole::Backup(2) => "backup-2",
            OracleRole::Backup(_) => "backup",
            OracleRole::Waiting   => "not elected",
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// SHA256(slot_hash || window_id) — unpredictable rotation entropy
fn rotation_hash(slot_hash: &[u8; 32], window_id: u64) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"strontium:rotation:v1:");
    h.update(slot_hash);
    h.update(&window_id.to_le_bytes());
    let result = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result[..32]);
    out
}

fn pubkey_short(pk: &[u8; 32]) -> String {
    let b58 = bs58::encode(pk).into_string();
    if b58.len() > 10 {
        format!("{}...{}", &b58[..6], &b58[b58.len()-4..])
    } else {
        b58
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_pubkey(seed: u8) -> [u8; 32] {
        let mut pk = [0u8; 32];
        pk[0] = seed;
        pk
    }

    #[test]
    fn test_solo_mode_below_min() {
        let mut rot = RotationState::new();
        rot.active_oracles = vec![mk_pubkey(1)];
        assert!(!rot.rotation_active());

        let d = rot.should_submit(&mk_pubkey(1), &[0u8; 32], 100, 0, 30, 30);
        assert!(d.should_submit);
        assert_eq!(d.role, OracleRole::Solo);
    }

    #[test]
    fn test_primary_submits_immediately() {
        let mut rot = RotationState::new();
        rot.active_oracles = vec![mk_pubkey(1), mk_pubkey(2)];

        let slot_hash = [42u8; 32];
        let window_id = 100u64;
        let primary   = rot.expected_primary(&slot_hash, window_id).unwrap();

        let d = rot.should_submit(&primary, &slot_hash, window_id, 0, 30, 30);
        assert!(d.should_submit);
        assert_eq!(d.role, OracleRole::Primary);
    }

    #[test]
    fn test_backup_waits_for_primary() {
        let mut rot = RotationState::new();
        rot.active_oracles = vec![mk_pubkey(1), mk_pubkey(2)];

        let slot_hash = [42u8; 32];
        let window_id = 100u64;
        let primary   = rot.expected_primary(&slot_hash, window_id).unwrap();
        let backup    = rot.expected_backup(&slot_hash, window_id, 1).unwrap();
        assert_ne!(primary, backup);

        // backup should NOT submit at t=0
        let d0 = rot.should_submit(&backup, &slot_hash, window_id, 0, 30, 30);
        assert!(!d0.should_submit, "backup should not submit at t=0");

        // backup SHOULD submit at t=30s
        let d30 = rot.should_submit(&backup, &slot_hash, window_id, 30, 30, 30);
        assert!(d30.should_submit, "backup should submit at t=30s");
        assert_eq!(d30.role, OracleRole::Backup(1));
    }

    #[test]
    fn test_refresh_sorts_and_dedupes() {
        let mut rot = RotationState::new();
        let subs = vec![
            (mk_pubkey(5), 100u64),
            (mk_pubkey(2), 99u64),
            (mk_pubkey(5), 98u64), // duplicate
        ];
        rot.refresh_from_submissions(&subs, 100, 50);
        assert_eq!(rot.active_oracles.len(), 2);
        assert!(rot.active_oracles[0] <= rot.active_oracles[1]);
    }

    #[test]
    fn test_empty_list_returns_solo() {
        let rot = RotationState::new();
        assert!(!rot.rotation_active());
        assert!(rot.expected_primary(&[0u8; 32], 0).is_none());
    }
}
