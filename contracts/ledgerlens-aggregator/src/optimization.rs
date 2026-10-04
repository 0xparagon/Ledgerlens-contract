/// Aggregator score read optimization for large wallet portfolios.
/// Reduces redundant storage queries and unnecessary computations when processing
/// wallets containing numerous asset pairs.

use soroban_sdk::{Address, Symbol, Vec};

/// Score read statistics for optimization tracking
#[derive(Debug, Clone)]
pub struct ScoreReadStats {
    /// Number of unique pairs queried
    pub total_pairs: u32,
    /// Number of cross-contract calls made
    pub cross_contract_calls: u32,
    /// Number of storage reads batched together
    pub batched_reads: u32,
    /// Estimated gas savings from batching (relative)
    pub gas_savings_percent: u32,
}

impl ScoreReadStats {
    /// Create new score read statistics
    pub fn new(total_pairs: u32) -> Self {
        ScoreReadStats {
            total_pairs,
            cross_contract_calls: 0,
            batched_reads: 0,
            gas_savings_percent: 0,
        }
    }

    /// Calculate the number of batches needed for the given pair count
    /// with a fixed batch size.
    pub fn calculate_batches(pair_count: u32, batch_size: u32) -> u32 {
        if batch_size == 0 {
            return pair_count;
        }
        (pair_count + batch_size - 1) / batch_size
    }

    /// Calculate estimated gas savings from batching.
    /// Each cross-contract call has a fixed overhead; batching reduces this.
    pub fn calculate_gas_savings(original_calls: u32, batched_calls: u32) -> u32 {
        if original_calls == 0 {
            return 0;
        }
        let reduction = original_calls.saturating_sub(batched_calls);
        // Approximate: each call has ~5000 gas overhead, so (reduction / original) * 100
        ((reduction * 100) / original_calls).min(100)
    }

    /// Update statistics after batching optimization
    pub fn apply_batching(&mut self, original_calls: u32, batched_calls: u32) {
        self.cross_contract_calls = batched_calls;
        self.batched_reads = original_calls;
        self.gas_savings_percent = Self::calculate_gas_savings(original_calls, batched_calls);
    }
}

/// Batch configuration for optimized reads
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Maximum number of pairs per batch
    pub batch_size: u32,
    /// Maximum number of parallel batches
    pub max_parallel: u32,
    /// Enable caching of results
    pub enable_caching: bool,
}

impl BatchConfig {
    /// Create default batch configuration
    pub fn default() -> Self {
        BatchConfig { batch_size: 10, max_parallel: 5, enable_caching: true }
    }

    /// Create configuration optimized for large portfolios
    pub fn optimized_for_large_portfolio() -> Self {
        BatchConfig { batch_size: 25, max_parallel: 10, enable_caching: true }
    }

    /// Validate batch configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.batch_size == 0 {
            return Err("batch_size must be greater than 0".to_string());
        }
        if self.max_parallel == 0 {
            return Err("max_parallel must be greater than 0".to_string());
        }
        Ok(())
    }
}

/// Represents a batched query result with scoring information
#[derive(Debug, Clone)]
pub struct BatchedScoreResult {
    /// Asset pair symbol
    pub asset_pair: Symbol,
    /// Score value
    pub score: u32,
    /// Whether the score is stale
    pub is_stale: bool,
}

/// Optimized portfolio scorer using batched reads
pub struct PortfolioScorer {
    config: BatchConfig,
    stats: ScoreReadStats,
}

impl PortfolioScorer {
    /// Create a new portfolio scorer with default configuration
    pub fn new(pair_count: u32) -> Self {
        Self::with_config(pair_count, BatchConfig::default())
    }

    /// Create a portfolio scorer with custom configuration
    pub fn with_config(pair_count: u32, config: BatchConfig) -> Self {
        if config.validate().is_err() {
            return PortfolioScorer {
                config: BatchConfig::default(),
                stats: ScoreReadStats::new(pair_count),
            };
        }

        let mut scorer = PortfolioScorer { config, stats: ScoreReadStats::new(pair_count) };

        // Calculate optimal batching
        let batches = ScoreReadStats::calculate_batches(pair_count, config.batch_size);
        let original_calls = pair_count;
        scorer.stats.apply_batching(original_calls, batches);

        scorer
    }

    /// Get the current statistics
    pub fn stats(&self) -> &ScoreReadStats {
        &self.stats
    }

    /// Get the batch size
    pub fn batch_size(&self) -> u32 {
        self.config.batch_size
    }

    /// Get the number of batches required
    pub fn batch_count(&self) -> u32 {
        ScoreReadStats::calculate_batches(self.stats.total_pairs, self.config.batch_size)
    }

    /// Check if caching is enabled
    pub fn caching_enabled(&self) -> bool {
        self.config.enable_caching
    }
}

/// Identifies a single cross-contract shard read. Every field that can
/// influence the returned score is part of the key so that two reads only
/// share a memo entry when they are guaranteed to observe the same shard
/// state: the shard contract, the subject being scored, the policy
/// parameters and the shard revision the read was taken against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardReadKey {
    /// Shard contract that owns the state being read
    pub shard: Address,
    /// Subject (wallet / account) the read is scoped to
    pub subject: Address,
    /// Policy parameters that influence the result
    pub policy_params: u64,
    /// Shard revision the read is pinned to
    pub revision: u32,
}

impl ShardReadKey {
    /// Build a fully-qualified memo key for a shard read.
    pub fn new(shard: Address, subject: Address, policy_params: u64, revision: u32) -> Self {
        ShardReadKey { shard, subject, policy_params, revision }
    }
}

/// Memo scope for repeated cross-contract reads.
///
/// `Invocation` memoises only for the duration of a single contract
/// invocation (memory-only). `Ledger` memoises across invocations within the
/// same ledger. Ledger scope is only sound when the shard revision is part of
/// the key and the shard cannot mutate between the memoised reads inside the
/// memo's lifetime; callers must pin the revision they read against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoScope {
    /// Memory-only, valid for one invocation
    Invocation,
    /// Temporary storage, valid across invocations in a ledger
    Ledger,
}

/// Memoization configuration, including the kill switch.
#[derive(Debug, Clone)]
pub struct MemoConfig {
    /// Whether memoization is enabled at all (kill switch)
    pub enabled: bool,
    /// Scope of the memo
    pub scope: MemoScope,
}

impl MemoConfig {
    /// Default memo configuration: enabled, invocation-scoped.
    pub fn default() -> Self {
        MemoConfig { enabled: true, scope: MemoScope::Invocation }
    }

    /// Ledger-scoped memoization, enabled.
    pub fn ledger_scoped() -> Self {
        MemoConfig { enabled: true, scope: MemoScope::Ledger }
    }

    /// Kill switch: disable memoization entirely.
    pub fn disabled() -> Self {
        MemoConfig { enabled: false, scope: MemoScope::Invocation }
    }

    /// Whether a read for `key` may be served from the memo.
    pub fn is_active(&self) -> bool {
        self.enabled
    }
}

/// In-memory memo of shard reads for the lifetime of a single invocation.
///
/// This never writes persistent storage: entries live only in the contract's
/// transient memory and are dropped when the invocation returns. Because the
/// key includes the shard revision, a read memoised here can only be reused
/// while the shard state it was taken against is unchanged.
#[derive(Debug, Clone)]
pub struct ShardReadMemo {
    config: MemoConfig,
    entries: Vec<(ShardReadKey, u32)>,
    hits: u32,
    misses: u32,
}

impl ShardReadMemo {
    /// Create a memo with the given configuration.
    pub fn new(config: MemoConfig) -> Self {
        ShardReadMemo { config, entries: Vec::new(), hits: 0, misses: 0 }
    }

    /// Whether memoization is currently active (kill switch respected).
    pub fn is_active(&self) -> bool {
        self.config.is_active()
    }

    /// Look up a previously memoised read. Returns `None` when memoization is
    /// disabled or the key has not been seen.
    pub fn get(&mut self, key: &ShardReadKey) -> Option<u32> {
        if !self.is_active() {
            return None;
        }
        for (k, v) in self.entries.iter() {
            if &k == key {
                self.hits += 1;
                return Some(v);
            }
        }
        self.misses += 1;
        None
    }

    /// Record the result of a shard read under `key`.
    pub fn put(&mut self, key: ShardReadKey, value: u32) {
        if !self.is_active() {
            return;
        }
        self.entries.push_back((key, value));
    }

    /// Number of memo hits observed.
    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// Number of memo misses observed.
    pub fn misses(&self) -> u32 {
        self.misses
    }

    /// Number of cross-contract calls avoided by memoization.
    pub fn calls_saved(&self) -> u32 {
        self.hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_read_stats_creation() {
        let stats = ScoreReadStats::new(100);
        assert_eq!(stats.total_pairs, 100);
        assert_eq!(stats.cross_contract_calls, 0);
    }

    #[test]
    fn test_batch_calculation_exact_division() {
        let batches = ScoreReadStats::calculate_batches(100, 10);
        assert_eq!(batches, 10);
    }

    #[test]
    fn test_batch_calculation_with_remainder() {
        let batches = ScoreReadStats::calculate_batches(100, 15);
        assert_eq!(batches, 7); // (100 + 15 - 1) / 15 = 114 / 15 = 7
    }

    #[test]
    fn test_batch_calculation_single_batch() {
        let batches = ScoreReadStats::calculate_batches(5, 100);
        assert_eq!(batches, 1);
    }

    #[test]
    fn test_gas_savings_calculation() {
        let savings = ScoreReadStats::calculate_gas_savings(100, 10);
        assert_eq!(savings, 90); // (90 / 100) * 100 = 90%
    }

    #[test]
    fn test_gas_savings_no_improvement() {
        let savings = ScoreReadStats::calculate_gas_savings(10, 10);
        assert_eq!(savings, 0);
    }

    #[test]
    fn test_gas_savings_zero_original() {
        let savings = ScoreReadStats::calculate_gas_savings(0, 10);
        assert_eq!(savings, 0);
    }

    #[test]
    fn test_batch_config_default() {
        let config = BatchConfig::default();
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.max_parallel, 5);
        assert!(config.enable_caching);
    }

    #[test]
    fn test_batch_config_large_portfolio() {
        let config = BatchConfig::optimized_for_large_portfolio();
        assert_eq!(config.batch_size, 25);
        assert_eq!(config.max_parallel, 10);
        assert!(config.enable_caching);
    }

    #[test]
    fn test_batch_config_validation_success() {
        let config = BatchConfig { batch_size: 10, max_parallel: 5, enable_caching: true };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_batch_config_validation_zero_batch_size() {
        let config = BatchConfig { batch_size: 0, max_parallel: 5, enable_caching: true };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_batch_config_validation_zero_parallel() {
        let config = BatchConfig { batch_size: 10, max_parallel: 0, enable_caching: true };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_portfolio_scorer_creation() {
        let scorer = PortfolioScorer::new(100);
        assert_eq!(scorer.stats().total_pairs, 100);
        assert_eq!(scorer.batch_size(), 10);
    }

    #[test]
    fn test_portfolio_scorer_batch_count() {
        let scorer = PortfolioScorer::new(100);
        assert_eq!(scorer.batch_count(), 10); // 100 pairs / 10 batch_size = 10
    }

    #[test]
    fn test_portfolio_scorer_large_portfolio() {
        let config = BatchConfig::optimized_for_large_portfolio();
        let scorer = PortfolioScorer::with_config(500, config);
        assert_eq!(scorer.batch_size(), 25);
        assert_eq!(scorer.batch_count(), 20); // (500 + 25 - 1) / 25 = 20
    }

    #[test]
    fn test_memo_config_kill_switch() {
        let config = MemoConfig::disabled();
        assert!(!config.is_active());
        let mut memo = ShardReadMemo::new(config);
        assert!(!memo.is_active());
    }

    #[test]
    fn test_memo_returns_none_when_disabled() {
        let mut memo = ShardReadMemo::new(MemoConfig::disabled());
        let key = ShardReadKey::new(Address::default(), Address::default(), 1, 7);
        memo.put(key.clone(), 42);
        assert_eq!(memo.get(&key), None);
        assert_eq!(memo.calls_saved(), 0);
    }

    #[test]
    fn test_memo_hit_avoids_repeat_read() {
        let mut memo = ShardReadMemo::new(MemoConfig::default());
        let key = ShardReadKey::new(Address::default(), Address::default(), 1, 7);
        assert_eq!(memo.get(&key), None);
        memo.put(key.clone(), 42);
        assert_eq!(memo.get(&key), Some(42));
        assert_eq!(memo.hits(), 1);
        assert_eq!(memo.calls_saved(), 1);
    }

    #[test]
    fn test_memo_key_includes_revision() {
        let mut memo = ShardReadMemo::new(MemoConfig::ledger_scoped());
        let shard = Address::default();
        let subject = Address::default();
        let k1 = ShardReadKey::new(shard.clone(), subject.clone(), 1, 7);
        let k2 = ShardReadKey::new(shard, subject, 1, 8);
        memo.put(k1.clone(), 42);
        // A different revision must not be served from the memo.
        assert_eq!(memo.get(&k2), None);
        assert_eq!(memo.get(&k1), Some(42));
    }

    #[test]
    fn test_memo_key_includes_policy_params() {
        let mut memo = ShardReadMemo::new(MemoConfig::ledger_scoped());
        let shard = Address::default();
        let subject = Address::default();
        let k1 = ShardReadKey::new(shard.clone(), subject.clone(), 1, 7);
        let k2 = ShardReadKey::new(shard, subject, 2, 7);
        memo.put(k1.clone(), 42);
        assert_eq!(memo.get(&k2), None);
        assert_eq!(memo.get(&k1), Some(42));
    }
}
