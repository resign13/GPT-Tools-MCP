use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPreset {
    #[default]
    Default,
    FullAccess,
}

impl PermissionPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::FullAccess => "full_access",
        }
    }

    pub fn restrictive(self, other: Self) -> Self {
        self.min(other)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationPolicy {
    #[default]
    Strict,
    Compatibility,
    Host,
}

impl IsolationPolicy {
    pub fn restrictive(self, other: Self) -> Self {
        if self == Self::Strict || other == Self::Strict {
            Self::Strict
        } else if self == Self::Compatibility || other == Self::Compatibility {
            Self::Compatibility
        } else {
            Self::Host
        }
    }
}

/// Kept separately from the display preset to retain legacy hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyRestrictions {
    Restricted,
    Trusted,
    Dangerous,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub preset: PermissionPreset,
    pub isolation: IsolationPolicy,
    pub network_allowed: bool,
    /// Legacy restrictions are retained in memory to preserve old limits, but
    /// are never serialized as part of the public permission status contract.
    #[serde(default, skip_serializing)]
    pub legacy: Option<LegacyRestrictions>,
}

impl ExecutionPolicy {
    /// Transitional adapter for callers that still pass version-zero settings.
    /// Keep legacy spellings confined to the compatibility boundary.
    pub fn legacy_config_value(&self) -> &'static str {
        match self.legacy {
            Some(LegacyRestrictions::Restricted) => "restricted",
            Some(LegacyRestrictions::Trusted) => "trusted",
            Some(LegacyRestrictions::Dangerous) => "dangerous",
            None => self.preset.as_str(),
        }
    }

    /// Configuration version, not the string spelling, distinguishes a new save
    /// from an old full_access value written by an earlier experimental build.
    pub fn from_config(
        value: &str,
        version: u32,
        isolation: IsolationPolicy,
    ) -> Result<Self, String> {
        if isolation == IsolationPolicy::Host && (version != 1 || value != "full_access") {
            return Err("Host execution requires an explicitly saved full_access policy (version 1).".into());
        }
        if version > 1 {
            return Err(format!("Unsupported permission policy version: {version}"));
        }
        let (preset, network_allowed, legacy) = match (version, value) {
            (1, "default") => (PermissionPreset::Default, true, None),
            (1, "full_access") => (PermissionPreset::FullAccess, true, None),
            (0, "safe" | "restricted" | "ask") => (
                PermissionPreset::Default,
                false,
                Some(LegacyRestrictions::Restricted),
            ),
            (0, "" | "default" | "trusted" | "developer" | "auto_approve") => (
                PermissionPreset::Default,
                true,
                Some(LegacyRestrictions::Trusted),
            ),
            (0, "dangerous" | "admin" | "full_access") => (
                PermissionPreset::FullAccess,
                true,
                Some(LegacyRestrictions::Dangerous),
            ),
            _ => return Err(format!("Unknown permission preset: {value}")),
        };
        Ok(Self {
            preset,
            isolation,
            network_allowed,
            legacy,
        })
    }

    pub fn host_access(&self) -> bool {
        self.preset == PermissionPreset::FullAccess && self.isolation == IsolationPolicy::Host && self.legacy.is_none()
    }

    pub fn auto_approves_soft_permissions(&self) -> bool {
        // Desktop approval is automatic for every preset, including migrated
        // configurations. Network denial and isolation remain separate limits.
        true
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let legacy_rank = |legacy| match legacy {
            Some(LegacyRestrictions::Restricted) => 0,
            Some(LegacyRestrictions::Trusted) => 1,
            Some(LegacyRestrictions::Dangerous) => 2,
            None => 3,
        };
        Self {
            preset: self.preset.restrictive(other.preset),
            isolation: self.isolation.restrictive(other.isolation),
            network_allowed: self.network_allowed && other.network_allowed,
            legacy: if legacy_rank(self.legacy) <= legacy_rank(other.legacy) {
                self.legacy
            } else {
                other.legacy
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_migration_preserves_legacy_limits() {
        for value in ["safe", "restricted", "ask"] {
            let p = ExecutionPolicy::from_config(value, 0, IsolationPolicy::Strict).unwrap();
            assert_eq!(p.preset.as_str(), "default");
            assert!(!p.network_allowed);
            assert!(p.auto_approves_soft_permissions());
        }
        for value in ["trusted", "developer", "auto_approve", ""] {
            let p = ExecutionPolicy::from_config(value, 0, IsolationPolicy::Strict).unwrap();
            assert_eq!(p.preset, PermissionPreset::Default);
            assert!(p.network_allowed);
            assert!(p.legacy.is_some());
            assert!(p.auto_approves_soft_permissions());
        }
        for value in ["dangerous", "admin", "full_access"] {
            let p = ExecutionPolicy::from_config(value, 0, IsolationPolicy::Strict).unwrap();
            assert_eq!(p.preset, PermissionPreset::FullAccess);
            assert!(p.auto_approves_soft_permissions());
        }
        assert!(ExecutionPolicy::from_config("unknown", 0, IsolationPolicy::Strict).is_err());
        assert!(ExecutionPolicy::from_config("default", 2, IsolationPolicy::Strict).is_err());
    }

    #[test]
    fn permission_intersection_preserves_stricter_dimensions() {
        let full =
            ExecutionPolicy::from_config("full_access", 1, IsolationPolicy::Compatibility).unwrap();
        let old = ExecutionPolicy::from_config("safe", 0, IsolationPolicy::Strict).unwrap();
        assert!(full.auto_approves_soft_permissions());
        let merged = full.intersect(&old);
        assert_eq!(merged, old.intersect(&full));
        assert_eq!(merged.preset, PermissionPreset::Default);
        assert_eq!(merged.isolation, IsolationPolicy::Strict);
        assert!(!merged.network_allowed);
        assert!(merged.auto_approves_soft_permissions());
        let default = ExecutionPolicy::from_config("default", 1, IsolationPolicy::Strict).unwrap();
        assert!(default.auto_approves_soft_permissions());
        assert!(full.intersect(&default).auto_approves_soft_permissions());
    }
}
