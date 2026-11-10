//! Number and key formatting for the Qt/QML layer.
//!
//! The core never produces user-facing prose: it hands the QML side numbers and
//! stable translation keys, and QML renders the sentence. That keeps the
//! translatable strings in one place (`i18nd`) and this crate free of a locale.

use crate::types::{PlanAction, PlanReason};

/// Unit of a byte count.
///
/// The web app's `formatBytes` divides by 1024 and labels the result `KiB`,
/// `MiB` and `GiB`; the same thresholds are kept here so both frontends show the
/// same number for the same package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteUnit {
    B,
    KiB,
    MiB,
    GiB,
}

impl ByteUnit {
    /// The unit as shown next to the number, without a leading space.
    pub fn symbol(self) -> &'static str {
        match self {
            ByteUnit::B => "B",
            ByteUnit::KiB => "KiB",
            ByteUnit::MiB => "MiB",
            ByteUnit::GiB => "GiB",
        }
    }
}

/// Formats a byte count the way the web app's `formatBytes` does.
///
/// Returns the value and its unit; the value is already rounded to the number of
/// decimals the web app's `toFixed` would use ([`byte_decimals`]), so QML can
/// render `i18nd("khaxbps", "%1 %2", value, unit)`. Negative values keep their
/// sign (the size of a removal is a negative disk delta) while the magnitude
/// picks the unit.
pub fn format_bytes(n: i64) -> (f64, ByteUnit) {
    let value = n as f64;
    let abs = n.unsigned_abs();
    let unit = if abs < 1024 {
        ByteUnit::B
    } else if abs < 1024 * 1024 {
        ByteUnit::KiB
    } else if abs < 1024 * 1024 * 1024 {
        ByteUnit::MiB
    } else {
        ByteUnit::GiB
    };
    let divisor = match unit {
        ByteUnit::B => 1.0,
        ByteUnit::KiB => 1024.0,
        ByteUnit::MiB => 1024.0 * 1024.0,
        ByteUnit::GiB => 1024.0 * 1024.0 * 1024.0,
    };
    (round_to(value / divisor, byte_decimals(n)), unit)
}

/// Number of decimals the web app prints for `n`: none below 1 KiB, one for
/// 1–10 KiB, none for 10 KiB–1 MiB, one for 1 MiB–1 GiB and two above that.
pub fn byte_decimals(n: i64) -> u8 {
    let abs = n.unsigned_abs();
    if abs < 1024 {
        0
    } else if abs < 1024 * 1024 {
        if abs < 10 * 1024 { 1 } else { 0 }
    } else if abs < 1024 * 1024 * 1024 {
        1
    } else {
        2
    }
}

fn round_to(value: f64, decimals: u8) -> f64 {
    let factor = 10f64.powi(i32::from(decimals));
    (value * factor).round() / factor
}

/// Translation key for a planned action: `install`, `remove`, `update`,
/// `reinstall`.
pub fn action_key(action: PlanAction) -> &'static str {
    match action {
        PlanAction::Install => "install",
        PlanAction::Remove => "remove",
        PlanAction::Update => "update",
        PlanAction::Reinstall => "reinstall",
    }
}

/// Translation key for a plan reason: `requested`, `dependency`, `reverse`,
/// `orphan`.
pub fn reason_key(reason: PlanReason) -> &'static str {
    match reason {
        PlanReason::Requested => "requested",
        PlanReason::Dependency => "dependency",
        PlanReason::Reverse => "reverse",
        PlanReason::Orphan => "orphan",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_below_one_kib_are_integers() {
        assert_eq!(format_bytes(0), (0.0, ByteUnit::B));
        assert_eq!(format_bytes(512), (512.0, ByteUnit::B));
        assert_eq!(format_bytes(-512), (-512.0, ByteUnit::B));
    }

    #[test]
    fn kib_uses_one_decimal_below_ten() {
        assert_eq!(format_bytes(1536), (1.5, ByteUnit::KiB));
        assert_eq!(byte_decimals(1536), 1);
    }

    #[test]
    fn kib_drops_decimals_at_ten_and_above() {
        assert_eq!(format_bytes(15 * 1024), (15.0, ByteUnit::KiB));
        assert_eq!(byte_decimals(15 * 1024), 0);
    }

    #[test]
    fn mib_and_gib_match_the_web_app() {
        assert_eq!(format_bytes(5 * 1024 * 1024), (5.0, ByteUnit::MiB));
        assert_eq!(byte_decimals(5 * 1024 * 1024), 1);
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), (3.0, ByteUnit::GiB));
        assert_eq!(byte_decimals(3 * 1024 * 1024 * 1024), 2);
    }

    #[test]
    fn negative_sizes_keep_their_sign() {
        assert_eq!(format_bytes(-1536), (-1.5, ByteUnit::KiB));
        assert_eq!(format_bytes(-2 * 1024 * 1024), (-2.0, ByteUnit::MiB));
    }

    #[test]
    fn units_are_labelled_like_the_web_app() {
        assert_eq!(ByteUnit::B.symbol(), "B");
        assert_eq!(ByteUnit::KiB.symbol(), "KiB");
        assert_eq!(ByteUnit::MiB.symbol(), "MiB");
        assert_eq!(ByteUnit::GiB.symbol(), "GiB");
    }

    #[test]
    fn keys_are_stable_translation_keys() {
        assert_eq!(action_key(PlanAction::Install), "install");
        assert_eq!(action_key(PlanAction::Remove), "remove");
        assert_eq!(action_key(PlanAction::Update), "update");
        assert_eq!(action_key(PlanAction::Reinstall), "reinstall");
        assert_eq!(reason_key(PlanReason::Requested), "requested");
        assert_eq!(reason_key(PlanReason::Dependency), "dependency");
        assert_eq!(reason_key(PlanReason::Reverse), "reverse");
        assert_eq!(reason_key(PlanReason::Orphan), "orphan");
    }
}
