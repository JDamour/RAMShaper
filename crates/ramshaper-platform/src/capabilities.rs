//! Capability reporting (NFR-06: typed capability discovery).

/// Global physical/commit memory counters are available.
pub const CAPABILITY_SYSTEM_MEMORY_COUNTERS: &str = "system_memory_counters";
/// The OS low-memory resource notification is available.
pub const CAPABILITY_LOW_MEMORY_NOTIFICATION: &str = "low_memory_notification";
/// Native process identity queries are available.
pub const CAPABILITY_PROCESS_IDENTITY: &str = "process_identity";

/// One capability: supported or not, with an explicit reason when absent.
/// Unsupported is reported, never silently skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInfo {
    /// Stable capability name.
    pub name: &'static str,
    /// Whether the capability is usable on this system.
    pub supported: bool,
    /// Why the capability is unavailable (required when `supported` is false).
    pub reason: Option<String>,
}

impl CapabilityInfo {
    /// A supported capability.
    #[must_use]
    pub fn supported(name: &'static str) -> Self {
        Self {
            name,
            supported: true,
            reason: None,
        }
    }

    /// An unsupported capability with the reason it is unavailable.
    #[must_use]
    pub fn unsupported(name: &'static str, reason: impl Into<String>) -> Self {
        Self {
            name,
            supported: false,
            reason: Some(reason.into()),
        }
    }
}

/// The capability set of one adapter instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    /// Reported capabilities.
    pub entries: Vec<CapabilityInfo>,
}

impl CapabilitySet {
    /// Empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a capability report.
    pub fn push(&mut self, info: CapabilityInfo) {
        self.entries.push(info);
    }

    /// Whether the named capability is supported.
    #[must_use]
    pub fn is_supported(&self, name: &str) -> bool {
        self.entries.iter().any(|c| c.name == name && c.supported)
    }

    /// The reason a capability is unsupported, if recorded.
    #[must_use]
    pub fn reason_for(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|c| c.name == name && !c.supported)
            .and_then(|c| c.reason.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_reports_support_and_reasons() {
        let mut set = CapabilitySet::new();
        set.push(CapabilityInfo::supported(CAPABILITY_SYSTEM_MEMORY_COUNTERS));
        set.push(CapabilityInfo::unsupported(
            CAPABILITY_LOW_MEMORY_NOTIFICATION,
            "CreateMemoryResourceNotification failed (Win32 1455)",
        ));
        assert!(set.is_supported(CAPABILITY_SYSTEM_MEMORY_COUNTERS));
        assert!(!set.is_supported(CAPABILITY_LOW_MEMORY_NOTIFICATION));
        assert_eq!(
            set.reason_for(CAPABILITY_LOW_MEMORY_NOTIFICATION),
            Some("CreateMemoryResourceNotification failed (Win32 1455)")
        );
        assert!(set.reason_for(CAPABILITY_PROCESS_IDENTITY).is_none());
    }
}
