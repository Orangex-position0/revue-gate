//! 领域服务：配额策略（QuotaPolicy）——纯业务判断，不碰 DB 与 HTTP。
//!
//! 判定规则：`quota.limit` 为 `Some` 且 `used >= limit` 视为超限（请求应返回 429）；
//! `limit` 为 `None`（无上限）恒放行。编排职责（查找密钥、持久化累加）在 usecases。

use super::api_key::Quota;

/// 配额判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCheck {
    /// 允许放行。
    Ok,
    /// 超过配额上限，应返回 429。
    Exceeded,
}

/// 配额策略领域服务：判定密钥配额是否超限。
pub struct QuotaPolicy;

impl QuotaPolicy {
    /// 校验配额：`used >= limit` 视为超限；`limit` 为 `None` 恒放行。
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

    /// 无上限（limit = None）时任意已用额度都应放行。
    #[test]
    fn unlimited_quota_always_allows() {
        assert_eq!(QuotaPolicy::check(&quota(None, 0)), QuotaCheck::Ok);
        assert_eq!(QuotaPolicy::check(&quota(None, 10_000)), QuotaCheck::Ok);
    }

    /// 未达到上限（used < limit）时放行。
    #[test]
    fn below_limit_allows() {
        assert_eq!(QuotaPolicy::check(&quota(Some(100), 0)), QuotaCheck::Ok);
        assert_eq!(QuotaPolicy::check(&quota(Some(100), 99)), QuotaCheck::Ok);
    }

    /// 达到或超过上限（used >= limit）时判定超限（边界值 100 本身即超限）。
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
