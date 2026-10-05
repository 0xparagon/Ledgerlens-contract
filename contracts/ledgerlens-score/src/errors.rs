use soroban_sdk::contracterror;

// XDR spec hard-limits contracterror enums to 50 variants.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidScore = 4,
    InvalidConfidence = 5,
    ScoreNotFound = 6,
    ContractPaused = 7,
    NoPendingAdminTransfer = 8,
    EmptyBatch = 9,
    BatchTooLarge = 10,
    ArithmeticOverflow = 11,
    UpgradeAlreadyPending = 12,
    NoPendingUpgrade = 13,
    InsufficientSigners = 14,
    UnauthorizedSigner = 15,
    InvalidThreshold = 16,
    ServiceSetFull = 17,
    SignerAlreadyInSet = 18,
    SignerNotInSet = 19,
    UpgradeNotReady = 20,
    InvalidUpgradeDelay = 21,
    InvalidStalenessWindow = 22,
    RateLimitExceeded = 23,
    InvalidCooldown = 24,
    InvalidTimestamp = 25,
    ServicePubkeyNotSet = 26,
    InvalidAttestation = 27,
    InvalidPubkeyLength = 28,
    InvalidHistoryDepth = 29,
    InsufficientConsensus = 30,
    ConsensusInputEmpty = 31,
    InvalidConsensusConfig = 32,
    AdminSetFull = 33,
    AdminSignerNotInSet = 34,
    InsufficientAdminSigners = 35,
    CyclicDelegation = 36,
    ScoreEmbargoed = 37,
    FeeTokenNotSet = 38,
    QuorumFailureWindowNotElapsed = 39,
    RevealWindowExpired = 40,
    CommitmentMismatch = 41,
    InvalidFinalityBuffer = 42,
    NoPendingScore = 43,
    FinalityWindowNotElapsed = 44,
    InvalidDisputeBond = 45,
    DisputeAlreadyOpen = 46,
    DisputeNotFound = 47,
    DisputeNotYetTimedOut = 48,
    InvalidHysteresisMargin = 49,
    InvalidModelPriorWeight = 50,
}

#[allow(non_upper_case_globals)]
impl Error {
    pub const InvalidMinConfidence: Error = Error::InvalidConfidence;
    pub const InvalidWithdrawalAmount: Error = Error::InvalidThreshold;
    pub const WithdrawalInProgress: Error = Error::Unauthorized;
    pub const PairPaused: Error = Error::ContractPaused;
    pub const PausedPairIndexFull: Error = Error::ServiceSetFull;
    pub const DelegateNotFound: Error = Error::ScoreNotFound;
    pub const InvalidDecayRate: Error = Error::InvalidThreshold;
    pub const CounterpartyLinkFull: Error = Error::ServiceSetFull;
    pub const CounterpartyNotFound: Error = Error::ScoreNotFound;
    pub const SelfLink: Error = Error::InvalidScore;
    pub const ScoreVelocityExceeded: Error = Error::RateLimitExceeded;
    pub const InvalidEscalation: Error = Error::InvalidThreshold;
    pub const InvalidJump: Error = Error::InvalidScore;
    pub const BelowScoreFloor: Error = Error::InvalidScore;
    pub const InvalidScoreFloorPolicy: Error = Error::InvalidThreshold;
    pub const DisputeIndexFull: Error = Error::ServiceSetFull;
    pub const ActorDisputeLimitExceeded: Error = Error::RateLimitExceeded;
    pub const EmbargoedWalletIndexFull: Error = Error::ServiceSetFull;
    /// `overlap_secs` outside `[0, MAX_KEY_OVERLAP_SECS]` in a key rotation.
    pub const InvalidKeyOverlap: Error = Error::InvalidThreshold;
    /// `secs` outside `[0, MAX_REVEAL_WINDOW_SECS]` in `set_reveal_window`.
    pub const InvalidRevealWindow: Error = Error::InvalidThreshold;

    pub const ModelVersionNotRegistered: Error = Error::InvalidScore;
    pub const ModelVersionDeprecated: Error = Error::Unauthorized;
    pub const ModelVersionAlreadyDeprecated: Error = Error::AlreadyInitialized;
    pub const ModelVersionAlreadyRegistered: Error = Error::SignerAlreadyInSet;
    pub const ModelVersionRegistryFull: Error = Error::ServiceSetFull;
    pub const ModelVersionNotReady: Error = Error::UpgradeNotReady;
    pub const ModelVersionAlreadyProposed: Error = Error::UpgradeAlreadyPending;
    pub const ModelVersionNotProposed: Error = Error::NoPendingUpgrade;
    pub const ModelVersionNotActive: Error = Error::Unauthorized;

    pub const NotFound: Error = Error::ScoreNotFound;
    pub const FeeRecipientNotSet: Error = Error::FeeTokenNotSet;
    pub const FeeRecipientMismatch: Error = Error::Unauthorized;

    // ── Adaptive Threshold ─────────────────────────────────────────────────
    /// Returned when an invalid target percentile is provided (must be 50-99).
    pub const InvalidPercentile: Error = Error::InvalidThreshold;

    pub const ParameterProposalNotFound: Error = Error::ScoreNotFound;
    pub const ParameterProposalNotReady: Error = Error::UpgradeNotReady;
    pub const ParameterProposalVetoPeriodEnded: Error = Error::QuorumFailureWindowNotElapsed;
    pub const ParameterProposalExpired: Error = Error::RevealWindowExpired;
    pub const TooManyPendingParameterProposals: Error = Error::ServiceSetFull;
    pub const ParameterProposalAlreadyExecuted: Error = Error::AlreadyInitialized;
    pub const ParameterProposalVetoed: Error = Error::DisputeAlreadyOpen;
    pub const InvalidParameterKey: Error = Error::InvalidThreshold;
    pub const InvalidParameterValue: Error = Error::InvalidScore;
    pub const InvalidParameterTimeLock: Error = Error::InvalidUpgradeDelay;

    pub const EpochClosed: Error = Error::ContractPaused;
    pub const InsufficientPairData: Error = Error::InsufficientConsensus;
    pub const GateCallerListFull: Error = Error::ServiceSetFull;
    pub const GateCallerNotInList: Error = Error::ScoreNotFound;
    pub const ParamChangeAlreadyPending: Error = Error::UpgradeAlreadyPending;

    // ── Memory-exhaustion guards (#612) ─────────────────────────────────────
    /// Returned by `submit_scores_batch_attested` when `signers.len()`
    /// exceeds the current service set size, i.e. more entries than could
    /// ever be legitimately required. Reused discriminant: the enum is
    /// already at the 50-variant XDR limit.
    pub const TooManySigners: Error = Error::ServiceSetFull;

    // ── Aggregator composability ────────────────────────────────────────────
    /// Returned by `ledgerlens-aggregator`'s `add_shard` when a candidate shard
    /// does not advertise the `ILedgerLensScore` capabilities the aggregator
    /// invokes across every shard. It signals that the shard's interface has
    /// drifted from the version the aggregator targets, so registering it would
    /// lead to failed or subtly incorrect cross-contract calls.
    pub const IncompatibleInterface: Error = Error::InvalidAttestation;

    // ── Architecture Governance & Reviewer Routing ─────────────────────────
    /// Returned when an architecture owner or reviewer address is invalid.
    pub const InvalidArchOwner: Error = Error::Unauthorized;
    /// Returned when trying to set more mandatory reviewers than MAX_MANDATORY_REVIEWERS (10).
    pub const MaxReviewersExceeded: Error = Error::ServiceSetFull;
    /// Returned when trying to add a duplicate mandatory reviewer address.
    pub const ReviewerAlreadyExists: Error = Error::SignerAlreadyInSet;
    /// Returned when a reviewer to be removed is not in the set.
    pub const ReviewerNotFound: Error = Error::SignerNotInSet;

    // ── Administrative capability partitioning (issue #695) ────────────────
    /// Returned by `set_policy_approval` when called with
    /// `Policy::DataDeletion`, which is configured via
    /// `set_deletion_approval_policy` instead.
    pub const InvalidPolicy: Error = Error::InvalidThreshold;

    // ── Permissionless keeper TTL reward pool ───────────────────────────────
    /// Returned when the keeper reward token has not been configured via
    /// `set_keeper_reward_token`.
    pub const KeeperRewardTokenNotSet: Error = Error::FeeTokenNotSet;
    /// Returned by `set_keeper_reward_params` when `reward_per_entry` exceeds
    /// `MAX_KEEPER_REWARD_PER_ENTRY` or `window_ledgers` exceeds
    /// `SCORE_TTL_THRESHOLD`.
    pub const InvalidKeeperRewardParams: Error = Error::InvalidThreshold;

    // ── Permissionless attested relay ───────────────────────────────────────
    /// Returned by `relay_attested_score` when `valid_before_ledger` is in
    /// the past, or the attestation's `contract_version` no longer matches —
    /// i.e. the attestation is no longer safe to relay.
    pub const StaleAttestation: Error = Error::InvalidAttestation;
    /// Returned when the relayer-tip token has not been configured via
    /// `set_relay_tip_token`.
    pub const RelayTipTokenNotSet: Error = Error::FeeTokenNotSet;

    // ── Prepaid gate-query credits ──────────────────────────────────────────
    /// Returned when `deposit_gate_credits`/`withdraw_gate_credits_request`
    /// is called with a non-positive amount, or a withdrawal request exceeds
    /// the depositor's current balance.
    pub const InvalidCreditAmount: Error = Error::InvalidScore;
    /// Returned when the gate-credit token has not been configured via
    /// `set_gate_credit_token`.
    pub const GateCreditTokenNotSet: Error = Error::FeeTokenNotSet;
    /// Returned by `query_risk_gate_metered` when the consumer's prepaid
    /// credit balance is below the computed fee for this call.
    pub const InsufficientGateCredits: Error = Error::RateLimitExceeded;
    /// Returned by `withdraw_gate_credits` when called before no request is
    /// pending, or before its unlock time has elapsed.
    pub const NoWithdrawalRequest: Error = Error::ScoreNotFound;
    pub const WithdrawalNotYetUnlocked: Error = Error::RateLimitExceeded;

    // ── Tiered gate fee schedule ─────────────────────────────────────────────
    /// Returned by `set_fee_tier_schedule` when tiers are not sorted in
    /// strictly ascending volume order, or any tier's fee exceeds the
    /// configured hard ceiling.
    pub const InvalidFeeTierSchedule: Error = Error::InvalidThreshold;
    /// Returned by `set_fee_exemption` when `expires_at` is not in the
    /// future relative to the current ledger timestamp.
    pub const InvalidExemptionExpiry: Error = Error::InvalidTimestamp;
}
