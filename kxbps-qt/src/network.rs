//! Bridge to `QNetworkInformation` through `src/network_shim.cpp`.
//!
//! The update check has to know whether a failed `xbps-install -M -un` means
//! "the repositories are unreachable" or "the system is offline". xbps prints
//! the same `Input/output error` and exits 0 either way, so Qt's platform
//! backend is asked instead. Nothing here interprets the number:
//! [`kxbps_core::update_check`] decides what `Disconnected` means, and the only
//! value this module adds is "no backend", which is not the same as offline.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("network_shim.h");

        // `int kxbps_network_reachability();` — the value of
        // `QNetworkInformation::Reachability`, or `-1` with no backend.
        #[cxx_name = "kxbps_network_reachability"]
        fn network_reachability() -> i32;
    }
}

/// The reachability reading the backend takes, as `QNetworkInformation::
/// Reachability` numbers: 0 Unknown, 1 Disconnected, 2 Local, 3 Site, 4 Online,
/// `-1` when no backend loaded.
///
/// A function value rather than the shim call so a test can answer without Qt.
pub type Reachability = dyn Fn() -> i32 + Send + Sync;

/// The shared, cloneable form the backend stores.
pub type ReachabilityFn = std::sync::Arc<Reachability>;

/// The shim-backed reading the application installs.
pub fn production_reachability() -> ReachabilityFn {
    std::sync::Arc::new(qobject::network_reachability)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `QNetworkInformation::Reachability::Disconnected`, as the shim spells it.
    const DISCONNECTED: i32 = 1;

    #[test]
    fn the_shim_answers_a_number_a_check_can_read() {
        // The value depends on this machine's NetworkManager; what matters is
        // that the call returns and lands in the documented range.
        let reading = production_reachability()();
        assert!(
            (-1..=4).contains(&reading),
            "reachability out of range: {reading}"
        );
    }

    #[test]
    fn a_disconnected_reading_is_the_one_a_check_reads_as_offline() {
        // The mapping itself is core's; this pins the number the shim and the
        // core agree on.
        assert!(kxbps_core::update_check::is_offline(DISCONNECTED));
        assert!(!kxbps_core::update_check::is_offline(0), "unknown tries");
        assert!(
            !kxbps_core::update_check::is_offline(-1),
            "no backend tries"
        );
    }
}
