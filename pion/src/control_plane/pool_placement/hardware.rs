//! Node hardware facts reported by heartbeat, and the requirement floor placement filters with.
//!
//! [`NodeHardwareCapabilities`] is parsed from a node's `capabilities_json` column, which heartbeat
//! ingest writes from [`parton::HostCapabilities`]. [`NodeHardwareRequirements`] is the constraint
//! side: it appears both inside a pool's stored selector and as the per-workload floor a caller
//! passes at resolution time.
//!
//! Disk capacity is deliberately absent. Volume headroom is a capacity-planning concern for the
//! orchestrator, computed from per-mount heartbeat data rather than pool membership.

use serde::{Deserialize, Serialize};

/// CPU architecture of a node, normalized across the spellings operators and agents use.
///
/// Parsing folds common aliases together so a selector written as `arm64` matches a node that
/// reports `aarch64` (the value `std::env::consts::ARCH` yields on Linux).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum CpuArchitecture {
    /// 64-bit x86 (`x86_64`, `amd64`, `x64`).
    X86_64,
    /// 64-bit ARM (`aarch64`, `arm64`).
    Aarch64,
    /// 32-bit ARM (`arm`, `armv7`, `armv7l`, `armhf`).
    Arm,
    /// Any other reported architecture, lowercased and trimmed.
    Other(String),
}

impl CpuArchitecture {
    /// Normalizes a reported or operator-written architecture string.
    ///
    /// Unrecognized values become [`CpuArchitecture::Other`] rather than an error, so a node on a
    /// new architecture still compares equal to a selector naming that same architecture.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "x86_64" | "amd64" | "x64" | "x86-64" => Self::X86_64,
            "aarch64" | "arm64" => Self::Aarch64,
            "arm" | "armv7" | "armv7l" | "armhf" => Self::Arm,
            _ => Self::Other(normalized),
        }
    }

    /// Canonical wire value, matching what a Parton agent reports for the known variants.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Arm => "arm",
            Self::Other(raw) => raw.as_str(),
        }
    }
}

impl std::fmt::Display for CpuArchitecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for CpuArchitecture {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

impl From<CpuArchitecture> for String {
    fn from(value: CpuArchitecture) -> Self {
        match value {
            CpuArchitecture::Other(raw) => raw,
            known => known.as_str().to_string(),
        }
    }
}

/// Hardware facts for one node, as last reported by its agent heartbeat.
///
/// Every field is optional because a heartbeat may predate a capability or report `0` for a value
/// it could not detect. `0` is treated as "not reported" rather than as a real CPU or memory
/// figure, so a node that fails to detect its RAM never appears to satisfy a memory floor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHardwareCapabilities {
    /// Reported CPU architecture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<CpuArchitecture>,
    /// Logical CPU count visible to the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_logical: Option<u32>,
    /// Total physical memory in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
}

impl NodeHardwareCapabilities {
    /// Reads hardware facts out of a node's `capabilities_json` column.
    ///
    /// Best effort by design: a malformed or partial payload yields absent fields instead of an
    /// error, and an unreported field only ever makes a node *less* eligible.
    #[must_use]
    pub fn from_capabilities_json(value: &serde_json::Value) -> Self {
        Self {
            arch: value
                .get("arch")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(CpuArchitecture::parse),
            cpu_logical: json_positive_u64(value.get("cpu_logical"))
                .and_then(|n| u32::try_from(n).ok()),
            memory_bytes: json_positive_u64(value.get("memory_bytes")),
        }
    }

    /// True when nothing about this node's hardware was reported.
    #[must_use]
    pub fn is_unreported(&self) -> bool {
        self.arch.is_none() && self.cpu_logical.is_none() && self.memory_bytes.is_none()
    }
}

fn json_positive_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    value
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        })
        .filter(|n| *n > 0)
}

/// Hardware floor a node must clear to be considered for placement.
///
/// An all-default value is *unconstrained*: it accepts every node, which is what keeps pools with
/// no stored selector behaving exactly as they did before selectors existed.
///
/// # JSON shape
///
/// ```json
/// { "arch": ["aarch64"], "min_cpu_logical": 4, "min_memory_bytes": 8589934592 }
/// ```
///
/// `arch` accepts a single string or an array, and `architectures` is accepted as a long-form
/// spelling of the same key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHardwareRequirements {
    /// Accepted architectures. Empty accepts any architecture.
    #[serde(
        default,
        rename = "arch",
        alias = "architectures",
        deserialize_with = "deserialize_architectures",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub architectures: Vec<CpuArchitecture>,
    /// Minimum logical CPU count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cpu_logical: Option<u32>,
    /// Minimum total memory in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_memory_bytes: Option<u64>,
}

impl NodeHardwareRequirements {
    /// True when this floor accepts every node.
    #[must_use]
    pub fn is_unconstrained(&self) -> bool {
        self.architectures.is_empty()
            && self.min_cpu_logical.is_none()
            && self.min_memory_bytes.is_none()
    }

    /// Returns the first requirement `capabilities` fails to meet, or `None` when it clears them
    /// all.
    ///
    /// A constrained field checked against an unreported capability fails. Placement should not
    /// send an `aarch64` image to a host whose architecture is unknown.
    #[must_use]
    pub fn unmet(&self, capabilities: &NodeHardwareCapabilities) -> Option<UnmetRequirement> {
        if !self.architectures.is_empty()
            && !capabilities
                .arch
                .as_ref()
                .is_some_and(|arch| self.architectures.contains(arch))
        {
            return Some(UnmetRequirement::Architecture {
                accepted: self.architectures.clone(),
                reported: capabilities.arch.clone(),
            });
        }
        if let Some(required) = self.min_cpu_logical {
            if capabilities.cpu_logical.is_none_or(|have| have < required) {
                return Some(UnmetRequirement::CpuLogical {
                    required,
                    reported: capabilities.cpu_logical,
                });
            }
        }
        if let Some(required) = self.min_memory_bytes {
            if capabilities.memory_bytes.is_none_or(|have| have < required) {
                return Some(UnmetRequirement::MemoryBytes {
                    required,
                    reported: capabilities.memory_bytes,
                });
            }
        }
        None
    }

    /// True when `capabilities` clears every requirement.
    #[must_use]
    pub fn is_satisfied_by(&self, capabilities: &NodeHardwareCapabilities) -> bool {
        self.unmet(capabilities).is_none()
    }
}

/// One requirement a node failed, kept typed so callers can report or branch on the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmetRequirement {
    /// Node architecture is absent from the accepted set.
    Architecture {
        /// Architectures the requirement accepts.
        accepted: Vec<CpuArchitecture>,
        /// What the node reported, if anything.
        reported: Option<CpuArchitecture>,
    },
    /// Node has too few logical CPUs, or reported none.
    CpuLogical {
        /// Minimum logical CPU count.
        required: u32,
        /// What the node reported, if anything.
        reported: Option<u32>,
    },
    /// Node has too little memory, or reported none.
    MemoryBytes {
        /// Minimum memory in bytes.
        required: u64,
        /// What the node reported, if anything.
        reported: Option<u64>,
    },
}

impl std::fmt::Display for UnmetRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Architecture { accepted, reported } => {
                let accepted = accepted
                    .iter()
                    .map(CpuArchitecture::as_str)
                    .collect::<Vec<_>>()
                    .join("|");
                write!(
                    f,
                    "arch {} != {accepted}",
                    reported_or_unknown(reported.as_ref())
                )
            }
            Self::CpuLogical { required, reported } => {
                write!(
                    f,
                    "cpu_logical {} < {required}",
                    reported_or_unknown(reported.as_ref())
                )
            }
            Self::MemoryBytes { required, reported } => {
                write!(
                    f,
                    "memory_bytes {} < {required}",
                    reported_or_unknown(reported.as_ref())
                )
            }
        }
    }
}

fn reported_or_unknown<T: std::fmt::Display>(reported: Option<&T>) -> String {
    reported.map_or_else(|| "unreported".to_string(), ToString::to_string)
}

fn deserialize_architectures<'de, D>(deserializer: D) -> Result<Vec<CpuArchitecture>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    /// Operators write `arch` as one string far more often than as a list, so accept both.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Absent,
        One(String),
        Many(Vec<String>),
    }

    let raw = match OneOrMany::deserialize(deserializer)? {
        OneOrMany::Absent => Vec::new(),
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    };
    Ok(raw
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(CpuArchitecture::parse)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(arch: &str, cpu: u32, memory: u64) -> NodeHardwareCapabilities {
        NodeHardwareCapabilities::from_capabilities_json(&serde_json::json!({
            "arch": arch,
            "cpu_logical": cpu,
            "memory_bytes": memory,
        }))
    }

    #[test]
    fn architecture_aliases_normalize_to_the_same_variant() {
        assert_eq!(CpuArchitecture::parse("amd64"), CpuArchitecture::X86_64);
        assert_eq!(CpuArchitecture::parse(" X86_64 "), CpuArchitecture::X86_64);
        assert_eq!(CpuArchitecture::parse("arm64"), CpuArchitecture::Aarch64);
        assert_eq!(CpuArchitecture::parse("AArch64"), CpuArchitecture::Aarch64);
        assert_eq!(CpuArchitecture::parse("armv7l"), CpuArchitecture::Arm);
        assert_eq!(
            CpuArchitecture::parse("riscv64"),
            CpuArchitecture::Other("riscv64".to_string())
        );
    }

    #[test]
    fn capabilities_parse_from_heartbeat_payload() {
        let parsed = caps("aarch64", 8, 16 * 1024 * 1024 * 1024);
        assert_eq!(parsed.arch, Some(CpuArchitecture::Aarch64));
        assert_eq!(parsed.cpu_logical, Some(8));
        assert_eq!(parsed.memory_bytes, Some(16 * 1024 * 1024 * 1024));
        assert!(!parsed.is_unreported());
    }

    #[test]
    fn zero_and_missing_capability_values_read_as_unreported() {
        let zeroed = caps("", 0, 0);
        assert!(zeroed.is_unreported());
        let empty = NodeHardwareCapabilities::from_capabilities_json(&serde_json::json!({}));
        assert!(empty.is_unreported());
        let wrong_types = NodeHardwareCapabilities::from_capabilities_json(&serde_json::json!({
            "arch": 7,
            "cpu_logical": "eight",
            "memory_bytes": [1],
        }));
        assert!(wrong_types.is_unreported());
    }

    #[test]
    fn default_requirements_accept_every_node() {
        let requirements = NodeHardwareRequirements::default();
        assert!(requirements.is_unconstrained());
        assert!(requirements.is_satisfied_by(&caps("x86_64", 2, 1024)));
        assert!(requirements.is_satisfied_by(&NodeHardwareCapabilities::default()));
    }

    #[test]
    fn architecture_requirement_matches_across_aliases() {
        let requirements = NodeHardwareRequirements {
            architectures: vec![CpuArchitecture::parse("arm64")],
            ..NodeHardwareRequirements::default()
        };
        assert!(requirements.is_satisfied_by(&caps("aarch64", 4, 1024)));
        assert!(!requirements.is_satisfied_by(&caps("x86_64", 4, 1024)));
    }

    #[test]
    fn constrained_requirement_rejects_unreported_hardware() {
        let unknown = NodeHardwareCapabilities::default();
        let arch_only = NodeHardwareRequirements {
            architectures: vec![CpuArchitecture::X86_64],
            ..NodeHardwareRequirements::default()
        };
        assert!(matches!(
            arch_only.unmet(&unknown),
            Some(UnmetRequirement::Architecture { .. })
        ));
        let cpu_only = NodeHardwareRequirements {
            min_cpu_logical: Some(2),
            ..NodeHardwareRequirements::default()
        };
        assert!(matches!(
            cpu_only.unmet(&unknown),
            Some(UnmetRequirement::CpuLogical { reported: None, .. })
        ));
        let memory_only = NodeHardwareRequirements {
            min_memory_bytes: Some(2048),
            ..NodeHardwareRequirements::default()
        };
        assert!(matches!(
            memory_only.unmet(&unknown),
            Some(UnmetRequirement::MemoryBytes { reported: None, .. })
        ));
    }

    #[test]
    fn minimums_are_inclusive_and_report_the_first_failure() {
        let requirements = NodeHardwareRequirements {
            architectures: vec![CpuArchitecture::X86_64],
            min_cpu_logical: Some(4),
            min_memory_bytes: Some(4096),
        };
        assert!(requirements.is_satisfied_by(&caps("x86_64", 4, 4096)));
        assert_eq!(
            requirements.unmet(&caps("x86_64", 3, 4096)),
            Some(UnmetRequirement::CpuLogical {
                required: 4,
                reported: Some(3)
            })
        );
        assert_eq!(
            requirements.unmet(&caps("x86_64", 4, 4095)),
            Some(UnmetRequirement::MemoryBytes {
                required: 4096,
                reported: Some(4095)
            })
        );
        // Architecture is checked first, so a wrong-arch node reports arch even when CPU also fails.
        assert!(matches!(
            requirements.unmet(&caps("aarch64", 1, 1)),
            Some(UnmetRequirement::Architecture { .. })
        ));
    }

    #[test]
    fn requirements_deserialize_single_arch_or_array() {
        let single: NodeHardwareRequirements =
            serde_json::from_value(serde_json::json!({ "arch": "arm64" }))
                .expect("single arch string parses");
        assert_eq!(single.architectures, vec![CpuArchitecture::Aarch64]);

        let many: NodeHardwareRequirements = serde_json::from_value(serde_json::json!({
            "architectures": ["x86_64", "arm64"],
            "min_cpu_logical": 2,
            "min_memory_bytes": 1024,
        }))
        .expect("arch array parses");
        assert_eq!(
            many.architectures,
            vec![CpuArchitecture::X86_64, CpuArchitecture::Aarch64]
        );
        assert_eq!(many.min_cpu_logical, Some(2));
        assert_eq!(many.min_memory_bytes, Some(1024));
    }

    #[test]
    fn requirements_ignore_blank_and_null_arch_entries() {
        let blank: NodeHardwareRequirements =
            serde_json::from_value(serde_json::json!({ "arch": "  " })).expect("blank arch parses");
        assert!(blank.is_unconstrained());
        let nulled: NodeHardwareRequirements =
            serde_json::from_value(serde_json::json!({ "arch": null })).expect("null arch parses");
        assert!(nulled.is_unconstrained());
        let listed: NodeHardwareRequirements =
            serde_json::from_value(serde_json::json!({ "arch": ["", "x86_64"] }))
                .expect("list with blank parses");
        assert_eq!(listed.architectures, vec![CpuArchitecture::X86_64]);
    }

    #[test]
    fn requirements_reject_malformed_minimums() {
        let err = serde_json::from_value::<NodeHardwareRequirements>(
            serde_json::json!({ "min_cpu_logical": "eight" }),
        )
        .expect_err("string cpu minimum must not be silently dropped");
        assert!(
            err.to_string().contains("invalid type"),
            "error should explain the type mismatch, got: {err}"
        );
    }

    #[test]
    fn requirements_round_trip_through_json() {
        let requirements = NodeHardwareRequirements {
            architectures: vec![CpuArchitecture::Aarch64],
            min_cpu_logical: Some(2),
            min_memory_bytes: None,
        };
        let json = serde_json::to_value(&requirements).expect("serialize requirements");
        assert_eq!(
            json,
            serde_json::json!({ "arch": ["aarch64"], "min_cpu_logical": 2 })
        );
        let parsed: NodeHardwareRequirements =
            serde_json::from_value(json).expect("re-parse requirements");
        assert_eq!(parsed, requirements);
    }

    #[test]
    fn unmet_requirement_display_names_the_reported_value() {
        let unmet = UnmetRequirement::Architecture {
            accepted: vec![CpuArchitecture::X86_64, CpuArchitecture::Aarch64],
            reported: None,
        };
        assert_eq!(unmet.to_string(), "arch unreported != x86_64|aarch64");
        let unmet = UnmetRequirement::CpuLogical {
            required: 8,
            reported: Some(2),
        };
        assert_eq!(unmet.to_string(), "cpu_logical 2 < 8");
    }
}
