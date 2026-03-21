#[derive(Debug, Clone)]
pub struct TaxConfig {
    /// Annual tax-free allowance (Freistellungsauftrag) in cents. Default: 100_100 (€1,001)
    pub freistellungsauftrag_cents: i64,
    /// Whether Kirchensteuer applies. Default: false
    pub kirchensteuer: bool,
    /// Kirchensteuer rate (0.08 or 0.09). Default: 0.09
    pub kirchensteuer_rate: f64,
}

impl Default for TaxConfig {
    fn default() -> Self {
        Self {
            freistellungsauftrag_cents: 100_100, // €1,001
            kirchensteuer: false,
            kirchensteuer_rate: 0.09,
        }
    }
}

/// Compute German capital gains tax on realized gains.
/// Applies Abgeltungssteuer (25%) + Solidaritätszuschlag (5.5% of 25%)
/// + optional Kirchensteuer.
/// Deducts Freistellungsauftrag from gains_cents before tax.
/// `accumulated_gains_cents`: total gains realized so far this year (for Freistellungsauftrag tracking)
/// Returns: tax_due_cents (0 if gains <= remaining allowance)
pub fn compute_tax(gains_cents: i64, accumulated_gains_cents: i64, cfg: &TaxConfig) -> i64 {
    if gains_cents <= 0 {
        return 0;
    }

    let remaining_allowance =
        (cfg.freistellungsauftrag_cents - accumulated_gains_cents).max(0);
    let taxable_gains = (gains_cents - remaining_allowance).max(0);

    if taxable_gains == 0 {
        return 0;
    }

    // Abgeltungssteuer: 25%
    let base_tax = taxable_gains * 25 / 100;
    // Solidaritätszuschlag: 5.5% of base_tax
    let soli = base_tax * 55 / 1_000;
    // Kirchensteuer (optional)
    let kirchensteuer = if cfg.kirchensteuer {
        taxable_gains * (cfg.kirchensteuer_rate * 100.0) as i64 / 10_000
    } else {
        0
    };

    base_tax + soli + kirchensteuer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_tax_within_allowance() {
        let cfg = TaxConfig::default();
        // Gains well within the €1,001 allowance
        let tax = compute_tax(50_000, 0, &cfg); // €500 gain, no prior gains
        assert_eq!(tax, 0);
    }

    #[test]
    fn test_tax_above_allowance() {
        let cfg = TaxConfig::default();
        // €2,000 gain with no prior accumulated gains → €999 taxable (above €1,001 allowance)
        let tax = compute_tax(200_000, 0, &cfg);
        // taxable = 200_000 - 100_100 = 99_900
        // base_tax = 99_900 * 25 / 100 = 24_975
        // soli = 24_975 * 55 / 1_000 = 1_373
        // total = 26_348
        assert_eq!(tax, 26_348);
    }

    #[test]
    fn test_tax_allowance_already_used() {
        let cfg = TaxConfig::default();
        // Allowance fully used; entire gain is taxable
        let tax = compute_tax(100_000, 200_000, &cfg); // €1,000 gain, already exceeded allowance
        // taxable = 100_000
        // base_tax = 25_000
        // soli = 1_375
        assert_eq!(tax, 26_375);
    }

    #[test]
    fn test_kirchensteuer() {
        let cfg = TaxConfig {
            freistellungsauftrag_cents: 0,
            kirchensteuer: true,
            kirchensteuer_rate: 0.09,
        };
        // €1,000 gain, no allowance
        let tax = compute_tax(100_000, 0, &cfg);
        // taxable = 100_000
        // base_tax = 25_000
        // soli = 1_375
        // kirchensteuer = 100_000 * 9 / 10_000 = 90
        assert_eq!(tax, 26_465);
    }

    #[test]
    fn test_zero_gain() {
        let cfg = TaxConfig::default();
        assert_eq!(compute_tax(0, 0, &cfg), 0);
        assert_eq!(compute_tax(-1000, 0, &cfg), 0);
    }
}
