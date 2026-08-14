//! Domain service: quota policy (QuotaPolicy) — pure business logic, no DB or HTTP.
//!
//! Rules: when `quota.limit` is `Some` and `used >= limit` it is over the limit (the request should return 429);
//! a `None` `limit` (no cap) always allows. Orchestration (finding the key, persisting the accumulation) lives in usecases.

use super::api_key::Quota;

/// Quota check result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCheck {
    /// Allowed.
    Ok,
    /// Exceeds the quota cap; should return 429.
    Exceeded,
}

/// Domain service for the quota policy: determines whether a key quota is over the limit.
pub struct QuotaPolicy;

impl QuotaPolicy {
    /// Check the quota: `used >= limit` is over the limit; a `None` `limit` always allows.
    pub fn check(quota: &Quota) -> QuotaCheck {
        match quota.limit {
            Some(limit) if quota.used >= limit => QuotaCheck::Exceeded,
            _ => QuotaCheck::Ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(limit: Option<u64>, used: u64) -> Quota {
        Quota { limit, used }
    }

    /// Any consumed quota is allowed when there is no cap (limit = None).
    #[test]
    fn unlimited_quota_always_allows() {
        assert_eq!(QuotaPolicy::check(&quota(None, 0)), QuotaCheck::Ok);
        assert_eq!(QuotaPolicy::check(&quota(None, 10_000)), QuotaCheck::Ok);
    }

    /// Allowed when below the cap (used < limit).
    #[test]
    fn below_limit_allows() {
        assert_eq!(QuotaPolicy::check(&quota(Some(100), 0)), QuotaCheck::Ok);
        assert_eq!(QuotaPolicy::check(&quota(Some(100), 99)), QuotaCheck::Ok);
    }

    /// At or over the cap (used >= limit) is judged over the limit (the boundary value 100 itself is over the limit).
    #[test]
    fn at_or_over_limit_exceeds() {
        assert_eq!(
            QuotaPolicy::check(&quota(Some(100), 100)),
            QuotaCheck::Exceeded
        );
        assert_eq!(
            QuotaPolicy::check(&quota(Some(100), 150)),
            QuotaCheck::Exceeded
        );
    }
}
