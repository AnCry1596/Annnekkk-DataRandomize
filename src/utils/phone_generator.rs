use crate::db::{DatabasePool, PhonePrefixDocument};
use anyhow::Result;
use rand::Rng;

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PhoneNumber {
    /// 10 raw digits, e.g. "2135550123"
    pub raw: String,
    /// "(213) 555-0123"
    pub formatted: String,
    /// "213-555-0123"
    pub dashed: String,
    /// "213.555.0123"
    pub dotted: String,
    /// "+12135550123"
    pub international: String,
}

// ---------------------------------------------------------------------------
// Pure builder (testable without DB)
// ---------------------------------------------------------------------------

pub fn build_phone(npa: &str, nxx: &str) -> PhoneNumber {
    let mut rng = rand::thread_rng();
    let line = format!("{:04}", rng.gen_range(0u32..10000));

    PhoneNumber {
        raw:           format!("{}{}{}", npa, nxx, line),
        formatted:     format!("({}) {}-{}", npa, nxx, line),
        dashed:        format!("{}-{}-{}", npa, nxx, line),
        dotted:        format!("{}.{}.{}", npa, nxx, line),
        international: format!("+1{}{}{}", npa, nxx, line),
    }
}

// ---------------------------------------------------------------------------
// Async generator — pulls NPA+NXX from DB
// ---------------------------------------------------------------------------

/// Generate a random phone number.
///
/// Priority: city → state → country → any prefix.
/// Each level falls through to the next if no match is found.
pub async fn generate_phone(
    pool: &DatabasePool,
    city: Option<&str>,
    state: Option<&str>,
    country: Option<&str>,
) -> Result<PhoneNumber> {
    let doc = if let Some(c) = city.filter(|s| !s.is_empty()) {
        PhonePrefixDocument::random_by_city(pool, c, state)
            .await?
            .or(if let Some(s) = state.filter(|s| !s.is_empty()) {
                PhonePrefixDocument::random_by_state(pool, s).await?
            } else { None })
            .or(if let Some(c) = country.filter(|s| !s.is_empty()) {
                PhonePrefixDocument::random_by_country(pool, c).await?
            } else { None })
            .or(PhonePrefixDocument::random(pool).await?)
    } else if let Some(s) = state.filter(|s| !s.is_empty()) {
        PhonePrefixDocument::random_by_state(pool, s)
            .await?
            .or(if let Some(c) = country.filter(|s| !s.is_empty()) {
                PhonePrefixDocument::random_by_country(pool, c).await?
            } else { None })
            .or(PhonePrefixDocument::random(pool).await?)
    } else if let Some(c) = country.filter(|s| !s.is_empty()) {
        PhonePrefixDocument::random_by_country(pool, c)
            .await?
            .or(PhonePrefixDocument::random(pool).await?)
    } else {
        PhonePrefixDocument::random(pool).await?
    };

    let (npa, nxx) = doc
        .map(|d| (d.npa, d.nxx))
        .unwrap_or_else(|| ("555".to_string(), "555".to_string()));

    Ok(build_phone(&npa, &nxx))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Unit tests — pure builder, no DB
    // -------------------------------------------------------------------------

    fn npa_of(p: &PhoneNumber) -> &str { &p.raw[..3] }
    fn nxx_of(p: &PhoneNumber) -> &str { &p.raw[3..6] }

    #[test]
    fn raw_is_10_digits() {
        let p = build_phone("213", "555");
        assert_eq!(p.raw.len(), 10);
        assert!(p.raw.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn npa_and_nxx_preserved() {
        let p = build_phone("415", "201");
        assert_eq!(npa_of(&p), "415");
        assert_eq!(nxx_of(&p), "201");
    }

    #[test]
    fn formatted_shape() {
        let p = build_phone("212", "555");
        assert!(p.formatted.starts_with('('));
        assert!(p.formatted.contains(") "));
        assert!(p.formatted.contains('-'));
    }

    #[test]
    fn dashed_has_two_dashes() {
        let p = build_phone("312", "444");
        assert_eq!(p.dashed.matches('-').count(), 2);
    }

    #[test]
    fn dotted_has_two_dots() {
        let p = build_phone("512", "333");
        assert_eq!(p.dotted.matches('.').count(), 2);
    }

    #[test]
    fn international_starts_with_plus_one() {
        let p = build_phone("617", "222");
        assert!(p.international.starts_with("+1"));
        assert_eq!(p.international.len(), 12);
    }
}
