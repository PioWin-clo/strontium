use anchor_lang::prelude::*;

declare_id!("2Z9ymNXMXjqMbDRj6NhPo7LLMaqdn2nfU1hvy19ScRAe");

pub const MAX_SUBMISSIONS:    usize = 32;
pub const RING_SIZE:          usize = 288;  // 24h at 300s interval (5min * 288 = 1440min = 24h)
pub const WINDOW_SLOTS:       u64   = 150;
pub const MIN_CONFIDENCE:     u8    = 60;
pub const MAX_SPREAD_MS:      i64   = 50;
pub const TTL_90_DAYS_SECS:   i64   = 90 * 24 * 3600;
pub const RENEW_WINDOW_SECS:  i64   = 7  * 24 * 3600;

pub const FLAG_OUTLIER:         u8 = 0b0000_0001;
pub const FLAG_LOW_CONFIDENCE:  u8 = 0b0000_0010;
pub const FLAG_SOURCE_MISMATCH: u8 = 0b0000_0100;

#[account]
pub struct ValidatorRegistration {
    pub oracle_keypair:          Pubkey,
    pub vote_account:            Pubkey,
    pub registered_at:           i64,
    pub expires_at_unix:         i64,
    pub last_health_check_slot:  u64,
    pub is_active:               bool,
    pub bump:                    u8,
    pub reliability_score:       u8,
    pub flags:                   u8,
    pub _pad:                    [u8; 4],
}

impl ValidatorRegistration {
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1 + 1 + 1 + 1 + 4;
}

#[zero_copy]
#[repr(C)]
pub struct ValidatorSubmission {
    pub validator:      Pubkey,
    pub timestamp_ms:   i64,
    pub spread_ms:      i64,
    pub slot:           u64,
    pub sources_used:   u8,
    pub confidence_pct: u8,
    pub sources_bitmap: u8,
    pub _pad:           [u8; 5],
}
// = 64 bytes


#[zero_copy]
#[repr(C)]
pub struct RingEntry {
    pub trusted_time_ms:  i64,
    pub slot:             u64,
    pub submitter_count:  u8,
    pub confidence_pct:   u8,
    pub spread_ms:        i16,
    pub sources_bitmap:   u8,
    pub _pad:             [u8; 3],
}
// = 24 bytes


#[account(zero_copy)]
#[repr(C)]
pub struct OracleState {
    pub authority:           Pubkey,
    pub bump:                u8,
    pub _pad0:               [u8; 7],
    pub trusted_time_ms:     i64,
    pub last_updated_slot:   u64,
    pub is_degraded:         u8,
    pub active_submitters:   u8,
    pub confidence_pct:      u8,
    pub quorum_threshold:    u8,
    pub spread_ms:           i16,
    pub _pad1:               [u8; 2],
    pub window_start_slot:   u64,
    pub submission_count:    u8,
    pub _pad2:               [u8; 7],
    pub ring_head:           u16,
    pub ring_count:          u16,
    pub _pad3:               [u8; 4],
    pub submissions:         [ValidatorSubmission; MAX_SUBMISSIONS],
    pub ring_buffer:         [RingEntry; RING_SIZE],
}

impl OracleState {
    pub const SIZE: usize = 8
        + 32 + 1 + 7
        + 8 + 8
        + 1 + 1 + 1 + 1 + 2 + 2
        + 8
        + 1 + 7
        + 2 + 2 + 4
        + MAX_SUBMISSIONS * 64
        + RING_SIZE * 24;

    pub fn is_stale(&self, slot: u64) -> bool {
        slot < self.window_start_slot || slot > self.window_start_slot + WINDOW_SLOTS * 2
    }

    pub fn find_slot(&self, validator: &Pubkey) -> Option<usize> {
        for i in 0..MAX_SUBMISSIONS {
            if &self.submissions[i].validator == validator { return Some(i); }
        }
        for i in 0..MAX_SUBMISSIONS {
            if self.submissions[i].validator == Pubkey::default() { return Some(i); }
        }
        None
    }

    pub fn aggregate(&mut self, current_slot: u64) {
        let mut timestamps    = [0i64; MAX_SUBMISSIONS];
        let mut count         = 0usize;
        let mut total_conf    = 0u32;
        let mut bitmap_union  = 0u8;

        for i in 0..MAX_SUBMISSIONS {
            let s = &self.submissions[i];
            if s.validator != Pubkey::default()
                && s.slot >= self.window_start_slot
                && s.slot <= current_slot
            {
                timestamps[count] = s.timestamp_ms;
                total_conf       += s.confidence_pct as u32;
                bitmap_union     |= s.sources_bitmap;
                count            += 1;
            }
        }

        self.active_submitters = count as u8;
        self.is_degraded = if (count as u8) < self.quorum_threshold { 1 } else { 0 };
        if count == 0 { return; }

        for i in 1..count {
            let key = timestamps[i];
            let mut j = i;
            while j > 0 && timestamps[j-1] > key {
                timestamps[j] = timestamps[j-1];
                j -= 1;
            }
            timestamps[j] = key;
        }

        let median = if count % 2 == 0 {
            let a = timestamps[count/2-1];
            let b = timestamps[count/2];
            a.checked_add(b).map(|s| s / 2).unwrap_or(a)
        } else {
            timestamps[count/2]
        };

        let spread   = ((timestamps[count-1] - timestamps[0]).clamp(0, i16::MAX as i64)) as i16;
        let avg_conf = (total_conf / count as u32) as u8;

        self.trusted_time_ms   = median;
        self.last_updated_slot = current_slot;
        self.spread_ms         = spread;
        self.confidence_pct    = avg_conf;

        let idx = self.ring_head as usize % RING_SIZE;
        self.ring_buffer[idx] = RingEntry {
            trusted_time_ms:  median,
            slot:             current_slot,
            submitter_count:  count as u8,
            confidence_pct:   avg_conf,
            spread_ms:        spread,
            sources_bitmap:   bitmap_union,
            _pad:             [0; 3],
        };
        self.ring_head = (self.ring_head + 1) % RING_SIZE as u16;
        if (self.ring_count as usize) < RING_SIZE { self.ring_count += 1; }
    }

    pub fn reset_window(&mut self, start_slot: u64) {
        self.window_start_slot = start_slot;
        self.submission_count  = 0;
        for i in 0..MAX_SUBMISSIONS {
            self.submissions[i] = ValidatorSubmission {
                validator:      Pubkey::default(),
                timestamp_ms:   0,
                spread_ms:      0,
                slot:           0,
                sources_used:   0,
                confidence_pct: 0,
                sources_bitmap: 0,
                _pad:           [0; 5],
            };
        }
    }
}

#[error_code]
pub enum StrontiumError {
    #[msg("NTP spread exceeds maximum (50ms)")]
    SpreadTooLarge,
    #[msg("NTP confidence below minimum (60%)")]
    ConfidenceTooLow,
    #[msg("Submission window is full (max 32 validators)")]
    SubmissionsFull,
    #[msg("Oracle is degraded — below quorum")]
    OracleDegraded,
    #[msg("Oracle data is stale")]
    OracleStale,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Validator not registered")]
    NotRegistered,
    #[msg("Validator registration is inactive")]
    RegistrationInactive,
    #[msg("Registration has expired — please renew")]
    RegistrationExpired,
    #[msg("Too early to renew — renew within 7 days of expiry")]
    TooEarlyToRenew,
    #[msg("Timestamp deviates >10s from on-chain clock — possible manipulation")]
    TimestampOutlier,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = authority, space = OracleState::SIZE, seeds = [b"strontium"], bump)]
    pub oracle_state:   AccountLoader<'info, OracleState>,
    #[account(mut)]
    pub authority:      Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RegisterSubmitter<'info> {
    #[account(mut)]
    pub oracle_keypair: Signer<'info>,
    pub vote_account:   Signer<'info>,
    #[account(
        init, payer = oracle_keypair,
        space = ValidatorRegistration::SIZE,
        seeds = [b"reg", oracle_keypair.key().as_ref()], bump
    )]
    pub registration:   Account<'info, ValidatorRegistration>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RenewRegistration<'info> {
    #[account(mut)]
    pub oracle_keypair: Signer<'info>,
    pub vote_account:   Signer<'info>,
    #[account(
        mut,
        seeds = [b"reg", oracle_keypair.key().as_ref()],
        bump  = registration.bump,
        constraint = registration.oracle_keypair == oracle_keypair.key() @ StrontiumError::Unauthorized,
        constraint = registration.vote_account   == vote_account.key()   @ StrontiumError::Unauthorized,
    )]
    pub registration: Account<'info, ValidatorRegistration>,
}

#[derive(Accounts)]
pub struct VerifyValidatorHealth<'info> {
    #[account(mut)]
    pub oracle_keypair: Signer<'info>,
    #[account(mut, seeds = [b"reg", oracle_keypair.key().as_ref()], bump = registration.bump)]
    pub registration:   Account<'info, ValidatorRegistration>,
}

#[derive(Accounts)]
pub struct DeregisterSubmitter<'info> {
    #[account(mut)]
    pub oracle_keypair: Signer<'info>,
    #[account(mut, seeds = [b"reg", oracle_keypair.key().as_ref()], bump = registration.bump)]
    pub registration:   Account<'info, ValidatorRegistration>,
}

#[derive(Accounts)]
pub struct SubmitTime<'info> {
    #[account(mut, seeds = [b"strontium"], bump)]
    pub oracle_state:   AccountLoader<'info, OracleState>,
    #[account(mut)]
    pub oracle_keypair: Signer<'info>,
    #[account(
        seeds = [b"reg", oracle_keypair.key().as_ref()],
        bump  = registration.bump,
        constraint = registration.is_active            @ StrontiumError::RegistrationInactive,
        constraint = registration.expires_at_unix > Clock::get()?.unix_timestamp @ StrontiumError::RegistrationExpired,
    )]
    pub registration: Account<'info, ValidatorRegistration>,
}

#[derive(Accounts)]
pub struct ReadTime<'info> {
    #[account(seeds = [b"strontium"], bump)]
    pub oracle_state: AccountLoader<'info, OracleState>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut, seeds = [b"strontium"], bump)]
    pub oracle_state: AccountLoader<'info, OracleState>,
    pub authority:    Signer<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct SubmitTimeArgs {
    pub timestamp_ms:   i64,
    pub spread_ms:      i64,
    pub sources_used:   u8,
    pub confidence_pct: u8,
    pub sources_bitmap: u8,
}

#[program]
pub mod strontium {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, quorum_threshold: u8) -> Result<()> {
        let mut state = ctx.accounts.oracle_state.load_init()?;
        let clock = Clock::get()?;
        state.authority         = ctx.accounts.authority.key();
        state.bump              = ctx.bumps.oracle_state;
        state.quorum_threshold  = quorum_threshold.max(1);
        state.window_start_slot = clock.slot;
        state.is_degraded       = 1;
        state.trusted_time_ms   = clock.unix_timestamp * 1000;
        msg!("Strontium initialized: quorum={} ring_size={}", quorum_threshold, RING_SIZE);
        Ok(())
    }

    pub fn register_submitter(ctx: Context<RegisterSubmitter>) -> Result<()> {
        let clock = Clock::get()?;
        let reg   = &mut ctx.accounts.registration;
        reg.oracle_keypair        = ctx.accounts.oracle_keypair.key();
        reg.vote_account          = ctx.accounts.vote_account.key();
        reg.registered_at         = clock.unix_timestamp;
        reg.expires_at_unix       = clock.unix_timestamp + TTL_90_DAYS_SECS;
        reg.last_health_check_slot = clock.slot;
        reg.is_active             = true;
        reg.bump                  = ctx.bumps.registration;
        reg.reliability_score     = 100;
        reg.flags                 = 0;
        msg!("Registered: oracle={} vote={} expires={}", reg.oracle_keypair, reg.vote_account, reg.expires_at_unix);
        Ok(())
    }

    pub fn renew_registration(ctx: Context<RenewRegistration>) -> Result<()> {
        let clock = Clock::get()?;
        let reg   = &mut ctx.accounts.registration;
        let time_to_expiry = reg.expires_at_unix - clock.unix_timestamp;
        require!(time_to_expiry < RENEW_WINDOW_SECS, StrontiumError::TooEarlyToRenew);
        reg.expires_at_unix        = clock.unix_timestamp + TTL_90_DAYS_SECS;
        reg.last_health_check_slot = clock.slot;
        reg.is_active              = true;
        reg.flags                  = 0;
        msg!("Renewed: oracle={} new_expiry={}", reg.oracle_keypair, reg.expires_at_unix);
        Ok(())
    }

    pub fn verify_validator_health(ctx: Context<VerifyValidatorHealth>) -> Result<()> {
        let clock = Clock::get()?;
        let reg   = &mut ctx.accounts.registration;
        reg.last_health_check_slot = clock.slot;
        reg.is_active              = true;
        Ok(())
    }

    pub fn deregister_submitter(ctx: Context<DeregisterSubmitter>) -> Result<()> {
        ctx.accounts.registration.is_active = false;
        msg!("Deregistered: oracle={}", ctx.accounts.oracle_keypair.key());
        Ok(())
    }

    pub fn submit_time(ctx: Context<SubmitTime>, args: SubmitTimeArgs) -> Result<()> {
        require!(args.spread_ms <= MAX_SPREAD_MS, StrontiumError::SpreadTooLarge);
        require!(args.confidence_pct >= MIN_CONFIDENCE, StrontiumError::ConfidenceTooLow);

        let mut state     = ctx.accounts.oracle_state.load_mut()?;
        let validator     = ctx.accounts.oracle_keypair.key();
        let clock         = Clock::get()?;

        if state.is_stale(clock.slot) { state.reset_window(clock.slot); }

        let idx = state.find_slot(&validator).ok_or(StrontiumError::SubmissionsFull)?;
        state.submissions[idx] = ValidatorSubmission {
            validator,
            timestamp_ms:   args.timestamp_ms,
            spread_ms:      args.spread_ms,
            sources_used:   args.sources_used,
            confidence_pct: args.confidence_pct,
            sources_bitmap: args.sources_bitmap,
            slot:           clock.slot,
            _pad:           [0; 5],
        };
        state.aggregate(clock.slot);
        msg!("submit: ts={}ms spread={}ms conf={} bitmap={}", args.timestamp_ms, args.spread_ms, args.confidence_pct, args.sources_bitmap);
        Ok(())
    }

    pub fn read_time(ctx: Context<ReadTime>, max_staleness_slots: u64) -> Result<i64> {
        let state = ctx.accounts.oracle_state.load()?;
        let clock = Clock::get()?;
        require!(state.is_degraded == 0, StrontiumError::OracleDegraded);
        require!(clock.slot.saturating_sub(state.last_updated_slot) <= max_staleness_slots, StrontiumError::OracleStale);
        Ok(state.trusted_time_ms)
    }

    pub fn update_config(ctx: Context<UpdateConfig>, quorum_threshold: u8) -> Result<()> {
        let mut state = ctx.accounts.oracle_state.load_mut()?;
        require!(state.authority == ctx.accounts.authority.key(), StrontiumError::Unauthorized);
        state.quorum_threshold = quorum_threshold.max(1);
        Ok(())
    }
}
