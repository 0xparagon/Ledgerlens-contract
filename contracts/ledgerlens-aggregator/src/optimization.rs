/// Aggregator score read optimization for large wallet portfolios.
/// Reduces redundant storage queries and unnecessary computations when processing
/// wallets containing numerous asset pairs.
///
/// # Extended-precision scores (issue #1155)
///
/// Scores are stored internally at basis-point resolution (`0..=10_000`) while the
/// public 0-100 API is preserved through a documented projection. The projection
/// uses **ceiling** rounding so that risk gates never under-report: a stored value
/// of `1` bp projects to `1`, and any non-zero risk projects to at least `1`.
/// Legacy 0-100 values are migrated implicitly by multiplying by `SCORE_SCALE`
/// (`100`), so no bulk storage rewrite is required.

use soroban_sdk::{Address, Symbol, Vec};

/// Basis-point resolution of the extended-precision score scale.
pub const SCORE_SCALE: u32 = 100;

/// Maximum value of the extended-precision (basis-point) score.
pub const SCORE_BP_MAX: u32 = 10_000;

/// Maximum value of the legacy 0-100 projected score.
pub const SCORE_LEGACY_MAX: u32 = 100;

/// Projection rounding rule applied when collapsing a basis-point score back to
/// the legacy 0-100 scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreRounding {
    /// Round toward zero. Never used for risk gates (can under-report).
    Floor,
    /// Round to the nearest integer, ties away from zero.
    Nearest,
    /// Round away from zero. Default: risk gates must never under-report.
    Ceiling,
}

/// Extended-precision score stored at basis-point resolution (`0..=10_000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedScore {
    /// Score in basis points.
    pub basis_points: u32,
}

impl ExtendedScore {
    /// Construct from a basis-point value, saturating at [`SCORE_BP_MAX`].
    pub fn from_basis_points(basis_points: u32) -> Self {
        ExtendedScore { basis_points: basis_points.min(SCORE_BP_MAX) }
    }

    /// Construct from a legacy 0-100 score using the implicit scale factor.
    /// This is the migration path for existing entries: no storage rewrite is
    /// needed, the legacy value is simply widened on read.
    pub fn from_legacy(legacy: u32) -> Self {
        Self::from_basis_points(legacy.min(SCORE_LEGACY_MAX).saturating_mul(SCORE_SCALE))
    }

    /// Project back to the legacy 0-100 scale using the given rounding rule.
    pub fn project(&self, rounding: ScoreRounding) -> u32 {
        let bp = self.basis_points;
        let projected = match rounding {
            ScoreRounding::Floor => bp / SCORE_SCALE,
            ScoreRounding::Nearest => (bp + SCORE_SCALE / 2) / SCORE_SCALE,
            ScoreRounding::Ceiling => (bp + SCORE_SCALE - 1) / SCORE_SCALE,
        };
        projected.min(SCORE_LEGACY_MAX)
    }

    /// Project using the default risk-safe rule (ceiling).
    pub fn to_legacy(&self) -> u32 {
        self.project(ScoreRounding::Ceiling)
    }
}

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

impl BatchedScoreResult {
    /// Project the stored basis-point score to the legacy 0-100 scale using the
    /// risk-safe (ceiling) rounding rule.
    pub fn projected_score(&self) -> u32 {
        ExtendedScore::from_basis_points(self.score).to_legacy()
    }
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
    fn test_legacy_roundtrip_is_identity() {
        // Property: any value originating from the legacy 0-100 scale must
        // project back to exactly the same value under every rounding rule.
        for legacy in 0..=SCORE_LEGACY_MAX {
            let extended = ExtendedScore::from_legacy(legacy);
            assert_eq!(extended.project(ScoreRounding::Floor), legacy);
            assert_eq!(extended.project(ScoreRounding::Nearest), legacy);
            assert_eq!(extended.project(ScoreRounding::Ceiling), legacy);
            assert_eq!(extended.to_legacy(), legacy);
        }
    }

    #[test]
    fn test_ceiling_never_under_reports() {
        // Any non-zero basis-point risk must project to at least 1.
        for bp in 1..=SCORE_BP_MAX {
            assert!(ExtendedScore::from_basis_points(bp).to_legacy() >= 1);
        }
        assert_eq!(ExtendedScore::from_basis_points(0).to_legacy(), 0);
    }

    #[test]
    fn test_rounding_bias_no_drift() {
        // Repeatedly widening and projecting must not drift: the ceiling rule
        // is idempotent on values that are multiples of SCORE_SCALE.
        let mut current = ExtendedScore::from_legacy(37);
        for _ in 0..1_000 {
            let projected = current.to_legacy();
            current = ExtendedScore::from_legacy(projected);
        }
        assert_eq!(current.to_legacy(), 37);
    }

    #[test]
    fn test_basis_points_saturate() {
        assert_eq!(ExtendedScore::from_basis_points(u32::MAX).basis_points, SCORE_BP_MAX);
        assert_eq!(ExtendedScore::from_legacy(u32::MAX).basis_points, SCORE_BP_MAX);
    }

    #[test]
    fn test_batched_result_projection() {
        let result = BatchedScoreResult {
            asset_pair: Symbol::short("XLM"),
            score: 1,
            is_stale: false,
        };
        assert_eq!(result.projected_score(), 1);
    }
}
