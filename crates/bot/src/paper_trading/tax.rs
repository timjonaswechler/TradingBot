use crate::config::TaxConfig;

pub struct TaxResult {
    pub tax:             i64, // fällige Steuer in Cent
    pub exemption_used:  i64, // genutzter Freistellungsauftrag in Cent
}

/// Berechnet die deutsche Kapitalertragsteuer auf einen realisierten Kursgewinn.
///
/// Steuern (Deutschland):
///   Abgeltungssteuer:    25,00 %
///   Solidaritätszuschlag: 5,50 % auf die Steuer (= 1,375 % vom Gewinn)
///   Kirchensteuer:        8,00 % auf die Steuer (optional)
///   Freistellungsauftrag: 1.000 € / Jahr steuerfrei (seit 2023)
pub fn calculate_tax(gain: i64, cfg: &TaxConfig, remaining_exemption: i64) -> TaxResult {
    if gain <= 0 {
        return TaxResult { tax: 0, exemption_used: 0 };
    }

    // Freistellungsauftrag anwenden
    let exempt  = gain.min(remaining_exemption);
    let taxable = gain - exempt;

    if taxable == 0 {
        return TaxResult { tax: 0, exemption_used: exempt };
    }

    // Abgeltungssteuer 25 %
    let mut tax = taxable / 4;

    // Solidaritätszuschlag 5,5 % auf die Steuer
    tax += tax * 55 / 1_000;

    // Kirchensteuer 8 % auf die Steuer (optional)
    if cfg.kirchensteuer {
        tax += tax * 8 / 100;
    }

    TaxResult { tax, exemption_used: exempt }
}
