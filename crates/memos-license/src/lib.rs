//! What the user is entitled to.
//!
//! One question — how many captures a week, and until when — answered in one
//! place. Before this the free allowance was a constant in the interface and
//! nothing could lift it; now it is a function of an entitlement, and there is
//! exactly one rule for reading it whether the plan came from a referral or,
//! later, from a payment.
//!
//! No I/O and no clock beyond `Utc::now`, on purpose. The entitlement arrives
//! from the API, is cached on disk, and has to be evaluated when neither is
//! reachable — a user on a plane is still on the plan they paid for.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The free plan's weekly allowance (§16).
pub const FREE_CAPTURES_PER_WEEK: u32 = 50;

/// What the app believes about the current plan.
///
/// `pro_until` is the whole of it. A date rather than a boolean because the
/// entitlement genuinely is one: a referral grants thirty days, a second grants
/// thirty more on the end of it, and "are they Pro" is a question about today
/// rather than a flag somebody has to remember to clear.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entitlement {
    #[serde(default)]
    pub pro_until: Option<DateTime<Utc>>,
    /// Months granted by referrals so far. Shown, not enforced — the date is
    /// what is enforced.
    #[serde(default)]
    pub months_earned: u32,
}

impl Entitlement {
    pub fn free() -> Self {
        Self::default()
    }

    pub fn pro_until(at: DateTime<Utc>) -> Self {
        Self {
            pro_until: Some(at),
            ..Self::default()
        }
    }

    /// Whether the plan is live right now.
    pub fn is_pro(&self) -> bool {
        self.pro_until.is_some_and(|at| at > Utc::now())
    }

    /// The weekly capture allowance, or `None` when there is no limit.
    ///
    /// `None` rather than `u32::MAX`: unlimited is not a very large number, and
    /// a meter that has to decide whether to draw a bar reads the difference
    /// between those two answers very differently.
    pub fn captures_per_week(&self) -> Option<u32> {
        if self.is_pro() {
            None
        } else {
            Some(FREE_CAPTURES_PER_WEEK)
        }
    }

    /// Whether a capture would exceed the allowance.
    ///
    /// The count is the local one, which can over-run slightly while offline —
    /// see `usage` in the store. Over-running is the correct way to be wrong: a
    /// capture that is refused is a thought that is lost, and a few extra on a
    /// free plan cost nothing anybody will ever notice.
    pub fn within_allowance(&self, used_this_week: u32) -> bool {
        match self.captures_per_week() {
            None => true,
            Some(limit) => used_this_week < limit,
        }
    }

    /// Days of Pro left, for the one sentence that reports it.
    ///
    /// Rounded up: with fourteen hours left a user has one day, not zero. Zero
    /// is what it says when the plan has actually run out.
    pub fn days_left(&self) -> Option<i64> {
        let at = self.pro_until?;
        let left = at - Utc::now();
        if left.num_seconds() <= 0 {
            return Some(0);
        }
        Some((left.num_seconds() + 86_399) / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn a_fresh_install_is_on_the_free_plan() {
        let free = Entitlement::free();
        assert!(!free.is_pro());
        assert_eq!(free.captures_per_week(), Some(FREE_CAPTURES_PER_WEEK));
        assert_eq!(free.days_left(), None);
    }

    #[test]
    fn pro_lifts_the_limit_entirely() {
        let pro = Entitlement::pro_until(Utc::now() + Duration::days(20));
        assert!(pro.is_pro());
        assert_eq!(pro.captures_per_week(), None);
        assert!(pro.within_allowance(10_000));
    }

    #[test]
    fn an_expired_plan_is_the_free_one_again() {
        let lapsed = Entitlement::pro_until(Utc::now() - Duration::seconds(1));
        assert!(!lapsed.is_pro());
        assert_eq!(lapsed.captures_per_week(), Some(FREE_CAPTURES_PER_WEEK));
        assert_eq!(lapsed.days_left(), Some(0));
    }

    #[test]
    fn the_allowance_runs_out_at_the_limit_not_after_it() {
        let free = Entitlement::free();
        assert!(free.within_allowance(FREE_CAPTURES_PER_WEEK - 1));
        assert!(!free.within_allowance(FREE_CAPTURES_PER_WEEK));
    }

    #[test]
    fn part_of_a_day_left_still_counts_as_a_day() {
        let nearly = Entitlement::pro_until(Utc::now() + Duration::hours(14));
        assert_eq!(nearly.days_left(), Some(1));
    }

    #[test]
    fn an_entitlement_survives_the_round_trip_it_is_cached_through() {
        let held = Entitlement {
            pro_until: Some(Utc::now() + Duration::days(9)),
            months_earned: 3,
        };
        let json = serde_json::to_string(&held).unwrap();
        assert_eq!(serde_json::from_str::<Entitlement>(&json).unwrap(), held);
    }

    #[test]
    fn a_cache_written_before_this_field_existed_still_reads() {
        let old: Entitlement = serde_json::from_str("{}").unwrap();
        assert_eq!(old, Entitlement::free());
    }
}
