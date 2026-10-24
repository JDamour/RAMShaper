//! Versioned TOML configuration: parse, validate, serialize (RS-030 config
//! portion; RS-012 inert profiles/exclusions; contract 14 §7; AT-08
//! core/config evidence).
//!
//! Rules implemented here:
//! - integer `schema_version` is required and must equal the supported
//!   version (future versions are rejected, not guessed);
//! - unknown fields are rejected with the field name;
//! - out-of-range values are rejected with a field-level error;
//! - the only mode in this slice is `observe_only` (startup mode; selecting
//!   a profile or importing configuration never grants action permission);
//! - a `[cohort] declared` label (`4gib`/`8gib`/`16gib`/`none`, AT-21) is
//!   accepted verbatim or rejected; it never changes behavior and is
//!   verified against the machine only at measurement time;
//! - a failed parse must not partially mutate settings: parsing produces a
//!   complete value or an error, so callers keep the last valid config.
//!
//! Atomic persistence lives in `ramshaper-windows` (needs OS rename
//! semantics); this module only parses, validates and serializes.

use serde::{Deserialize, Serialize};

use crate::cohort::HardwareCohort;

/// Currently supported configuration schema version.
pub const CONFIG_SCHEMA_VERSION: i64 = 1;

/// Startup mode. `ObserveOnly` is the only mode that exists in this slice;
/// any other mode string is a field-level error, never a silent default.
pub const MODE_OBSERVE_ONLY: &str = "observe_only";

/// Validated watch settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchSettings {
    /// Watch poll interval, milliseconds (100..=3,600,000).
    pub interval_ms: u64,
    /// System snapshot retention window, milliseconds (1,000..=86,400,000).
    pub retention_ms: u64,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            interval_ms: crate::watch::DEFAULT_INTERVAL_MS,
            retention_ms: crate::history::DEFAULT_RETENTION_MS,
        }
    }
}

/// Session-agent settings (RS-030). The pipe name is optional: the default
/// name is derived at runtime to include the logon session id so two
/// sessions cannot collide on one pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    /// Explicit pipe name override (`\\.\pipe\...`). When absent the agent
    /// derives `\\.\pipe\ramshaper-agent-s<session>-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe_name: Option<String>,
}

/// AT-21 cohort settings (RS-025, observe-only). The declaration is a
/// measurement label only: it never changes collection behavior. A declared
/// cohort is verified against the machine's measured memory at measurement
/// time ([`crate::cohort::verify_declared`]); `none` declares no cohort, so
/// observations classify as non-cohort baselines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortSettings {
    /// Declared hardware cohort: `4gib`, `8gib`, `16gib` or `none`.
    pub declared: HardwareCohort,
}

impl Default for CohortSettings {
    fn default() -> Self {
        Self {
            declared: HardwareCohort::NonCohort,
        }
    }
}

/// Profile presets (doc 06 §S04). All presets share the same defaults in
/// this slice (contract 14: "all profiles initially share these defaults");
/// selecting one is a stored label: it never changes collection behavior and
/// never grants permission to write process settings (contract 14 §5).
pub const PROFILES: [&str; 5] = ["balanced", "low_ram", "focus", "gaming_observe", "custom"];
/// Default profile.
pub const PROFILE_DEFAULT: &str = "balanced";

/// Inert profile selection (RS-012). The active preset is a label; policy
/// differentiation arrives with the G3 slices and is out of scope here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    /// Selected preset name; one of [`PROFILES`].
    #[serde(default = "default_profile")]
    pub active: String,
}

fn default_profile() -> String {
    PROFILE_DEFAULT.to_string()
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self {
            active: default_profile(),
        }
    }
}

/// User exclusions (RS-012, inert). Executable names the user never wants
/// enrolled for any future action; nothing actuates in this build, so the
/// list is stored and validated only. The mandatory exclusions of contract
/// 14 §5 (audio, security, accessibility-critical, install/update,
/// game/anti-cheat, RAMShaper itself) are eligibility rules, not config
/// values, so an import cannot weaken them by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExclusionSettings {
    /// Executable names (not paths), stored verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processes: Vec<String>,
}

/// Maximum number of user exclusions.
pub const MAX_EXCLUSIONS: usize = 512;
/// Maximum length of one exclusion entry (a file name, not a path).
pub const MAX_EXCLUSION_LEN: usize = 260;

/// The whole validated configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    /// Schema version; must equal [`CONFIG_SCHEMA_VERSION`].
    pub schema_version: i64,
    /// Operating mode; only `observe_only` exists in this slice.
    pub mode: String,
    /// Watch settings.
    pub watch: WatchSettings,
    /// Session-agent settings.
    #[serde(default)]
    pub agent: AgentSettings,
    /// AT-21 hardware-cohort declaration (defaults to `none`).
    #[serde(default)]
    pub cohort: CohortSettings,
    /// Inert profile selection (defaults to `balanced`).
    #[serde(default)]
    pub profiles: ProfileSettings,
    /// Inert user exclusions (defaults to empty).
    #[serde(default)]
    pub exclusions: ExclusionSettings,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            mode: MODE_OBSERVE_ONLY.to_string(),
            watch: WatchSettings::default(),
            agent: AgentSettings::default(),
            cohort: CohortSettings::default(),
            profiles: ProfileSettings::default(),
            exclusions: ExclusionSettings::default(),
        }
    }
}

/// Field-level configuration error (never a bare string over the wire).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The TOML text could not be parsed.
    #[error("configuration is not valid TOML: {0}")]
    Malformed(String),
    /// `schema_version` missing.
    #[error("field 'schema_version' is required")]
    MissingSchemaVersion,
    /// `schema_version` is not the supported version.
    #[error("unsupported schema_version {found}; this build supports {supported}")]
    UnsupportedSchemaVersion {
        /// Version found in the file.
        found: i64,
        /// Version this build supports.
        supported: i64,
    },
    /// An unknown field was present (config files are closed schemas).
    #[error("unknown field '{field}'")]
    UnknownField {
        /// The rejected field name.
        field: String,
    },
    /// A field value is out of range.
    #[error("field '{field}' out of range: {reason}")]
    OutOfRange {
        /// The rejected field name (dot-path).
        field: String,
        /// Why the value is invalid.
        reason: String,
    },
    /// The mode is not supported in this slice.
    #[error("unsupported mode '{found}'; this build only supports '{supported}'")]
    UnsupportedMode {
        /// Mode found in the file.
        found: String,
        /// The only supported mode.
        supported: &'static str,
    },
    /// A required field is missing.
    #[error("field '{field}' is required")]
    MissingField {
        /// The missing field name.
        field: String,
    },
    /// A `[profiles] active` string is not one of the known presets.
    #[error("unsupported profile '{found}'; valid profiles: {}", PROFILES.join(", "))]
    UnsupportedProfile {
        /// The rejected profile name.
        found: String,
    },
    /// A `[cohort] declared` string is not a valid cohort declaration.
    #[error(
        "unsupported cohort declaration '{found}'; valid declarations: {}",
        crate::cohort::HardwareCohort::VALID_DECLARATIONS
    )]
    UnsupportedCohort {
        /// The rejected declaration string.
        found: String,
    },
}

impl CoreConfig {
    /// Parse and validate a configuration document. Unknown fields are
    /// surfaced from TOML's own deserializer and normalized to
    /// [`ConfigError::UnknownField`].
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: Option<i64>,
            mode: Option<String>,
            #[serde(default)]
            watch: Option<toml::Table>,
            #[serde(default)]
            agent: Option<toml::Table>,
            #[serde(default)]
            cohort: Option<toml::Table>,
            #[serde(default)]
            profiles: Option<toml::Table>,
            #[serde(default)]
            exclusions: Option<toml::Table>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawWatch {
            interval_ms: Option<u64>,
            retention_ms: Option<u64>,
        }
        #[derive(Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        struct RawAgent {
            pipe_name: Option<String>,
        }
        #[derive(Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        struct RawCohort {
            declared: Option<String>,
        }
        #[derive(Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        struct RawProfiles {
            active: Option<String>,
        }
        #[derive(Deserialize, Default)]
        #[serde(deny_unknown_fields)]
        struct RawExclusions {
            processes: Option<Vec<String>>,
        }

        let raw: Raw = toml::from_str(text).map_err(|e| classify_toml_error(&e))?;
        let schema_version = raw
            .schema_version
            .ok_or(ConfigError::MissingSchemaVersion)?;
        if schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion {
                found: schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        let mode = raw.mode.unwrap_or_else(|| MODE_OBSERVE_ONLY.to_string());
        if mode != MODE_OBSERVE_ONLY {
            return Err(ConfigError::UnsupportedMode {
                found: mode,
                supported: MODE_OBSERVE_ONLY,
            });
        }
        let defaults = WatchSettings::default();
        let watch = match raw.watch {
            None => defaults,
            Some(table) => {
                let raw_watch: RawWatch = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| prefix_watch_field(classify_toml_error(&e)))?;
                WatchSettings {
                    interval_ms: raw_watch.interval_ms.unwrap_or(defaults.interval_ms),
                    retention_ms: raw_watch.retention_ms.unwrap_or(defaults.retention_ms),
                }
            }
        };
        validate_watch(&watch)?;
        let agent = match raw.agent {
            None => AgentSettings::default(),
            Some(table) => {
                let raw_agent: RawAgent = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| prefix_table_field("agent", classify_toml_error(&e)))?;
                AgentSettings {
                    pipe_name: raw_agent.pipe_name,
                }
            }
        };
        let cohort = match raw.cohort {
            None => CohortSettings::default(),
            Some(table) => {
                let raw_cohort: RawCohort = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| prefix_table_field("cohort", classify_toml_error(&e)))?;
                CohortSettings {
                    declared: match raw_cohort.declared {
                        None => HardwareCohort::NonCohort,
                        Some(text) => HardwareCohort::parse_declaration(&text)
                            .map_err(|_| ConfigError::UnsupportedCohort { found: text })?,
                    },
                }
            }
        };
        let profiles = match raw.profiles {
            None => ProfileSettings::default(),
            Some(table) => {
                let raw_profiles: RawProfiles = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| prefix_table_field("profiles", classify_toml_error(&e)))?;
                ProfileSettings {
                    active: raw_profiles
                        .active
                        .unwrap_or_else(|| PROFILE_DEFAULT.to_string()),
                }
            }
        };
        if !PROFILES.contains(&profiles.active.as_str()) {
            return Err(ConfigError::UnsupportedProfile {
                found: profiles.active,
            });
        }
        let exclusions = match raw.exclusions {
            None => ExclusionSettings::default(),
            Some(table) => {
                let raw_exclusions: RawExclusions = toml::Value::Table(table)
                    .try_into()
                    .map_err(|e| prefix_table_field("exclusions", classify_toml_error(&e)))?;
                ExclusionSettings {
                    processes: raw_exclusions.processes.unwrap_or_default(),
                }
            }
        };
        validate_exclusions(&exclusions)?;
        Ok(Self {
            schema_version,
            mode,
            watch,
            agent,
            cohort,
            profiles,
            exclusions,
        })
    }

    /// Serialize the configuration to TOML (round-trips through `parse`).
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self)
            .map_err(|e| ConfigError::Malformed(format!("serialization failed: {e}")))
    }
}

fn validate_watch(watch: &WatchSettings) -> Result<(), ConfigError> {
    if !(crate::watch::MIN_INTERVAL_MS..=crate::watch::MAX_INTERVAL_MS).contains(&watch.interval_ms)
    {
        return Err(ConfigError::OutOfRange {
            field: "watch.interval_ms".to_string(),
            reason: format!(
                "{} not in {}..={}",
                watch.interval_ms,
                crate::watch::MIN_INTERVAL_MS,
                crate::watch::MAX_INTERVAL_MS
            ),
        });
    }
    if !(1_000..=86_400_000).contains(&watch.retention_ms) {
        return Err(ConfigError::OutOfRange {
            field: "watch.retention_ms".to_string(),
            reason: format!("{} not in 1000..=86400000", watch.retention_ms),
        });
    }
    Ok(())
}

/// Validate the inert user-exclusion list: names only (no paths, no
/// control characters), bounded count and length. Stored verbatim; matching
/// semantics are a policy-slice concern (G3), not a config concern.
fn validate_exclusions(exclusions: &ExclusionSettings) -> Result<(), ConfigError> {
    if exclusions.processes.len() > MAX_EXCLUSIONS {
        return Err(ConfigError::OutOfRange {
            field: "exclusions.processes".to_string(),
            reason: format!(
                "{} entries exceeds the limit of {MAX_EXCLUSIONS}",
                exclusions.processes.len()
            ),
        });
    }
    for (index, name) in exclusions.processes.iter().enumerate() {
        let field = format!("exclusions.processes[{index}]");
        if name.is_empty() {
            return Err(ConfigError::OutOfRange {
                field,
                reason: "an exclusion must name an executable".to_string(),
            });
        }
        if name.len() > MAX_EXCLUSION_LEN {
            return Err(ConfigError::OutOfRange {
                field,
                reason: format!("{} exceeds {MAX_EXCLUSION_LEN} characters", name.len()),
            });
        }
        if name.contains(['/', '\\']) || name.chars().any(|c| c.is_control()) {
            return Err(ConfigError::OutOfRange {
                field,
                reason: "an exclusion is an executable name, not a path".to_string(),
            });
        }
    }
    Ok(())
}

/// Prefix nested-table field errors with the table path.
fn prefix_table_field(table: &str, error: ConfigError) -> ConfigError {
    match error {
        ConfigError::UnknownField { field } => ConfigError::UnknownField {
            field: format!("{table}.{field}"),
        },
        ConfigError::MissingField { field } => ConfigError::MissingField {
            field: format!("{table}.{field}"),
        },
        ConfigError::OutOfRange { field, reason } => ConfigError::OutOfRange {
            field: format!("{table}.{field}"),
            reason,
        },
        other => other,
    }
}

/// Prefix nested `[watch]` field errors with the table path.
fn prefix_watch_field(error: ConfigError) -> ConfigError {
    prefix_table_field("watch", error)
}

/// Normalize toml crate errors into field-level `ConfigError`s.
fn classify_toml_error(error: &toml::de::Error) -> ConfigError {
    let text = error.to_string();
    if let Some(field) = text.split("unknown field `").nth(1).and_then(|rest| {
        let end = rest.find('`')?;
        Some(rest[..end].to_string())
    }) {
        return ConfigError::UnknownField { field };
    }
    if text.contains("missing field") {
        let field = text
            .split("missing field `")
            .nth(1)
            .and_then(|rest| rest.find('`').map(|end| rest[..end].to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        return ConfigError::MissingField { field };
    }
    ConfigError::Malformed(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "\
schema_version = 1
mode = \"observe_only\"

[watch]
interval_ms = 2000
retention_ms = 900000
";

    #[test]
    fn valid_document_parses_with_every_field() {
        let config = CoreConfig::parse(VALID).expect("valid");
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.mode, "observe_only");
        assert_eq!(config.watch.interval_ms, 2000);
        assert_eq!(config.watch.retention_ms, 900_000);
    }

    #[test]
    fn roundtrip_through_serialize_and_parse() {
        let config = CoreConfig::parse(VALID).expect("valid");
        let text = config.to_toml().expect("ser");
        let back = CoreConfig::parse(&text).expect("re-parse");
        assert_eq!(config, back);
    }

    #[test]
    fn defaults_are_valid_and_observe_only() {
        let config = CoreConfig::default();
        assert_eq!(config.mode, MODE_OBSERVE_ONLY);
        let text = config.to_toml().expect("ser");
        assert_eq!(CoreConfig::parse(&text).expect("re-parse"), config);
    }

    #[test]
    fn unknown_fields_are_rejected_by_name() {
        let text = VALID.replace(
            "mode = \"observe_only\"",
            "mode = \"observe_only\"\nblorple = 3",
        );
        assert_eq!(
            CoreConfig::parse(&text),
            Err(ConfigError::UnknownField {
                field: "blorple".to_string()
            })
        );
        let nested = VALID.replace("interval_ms = 2000", "interval_ms = 2000\nbogus = true");
        assert_eq!(
            CoreConfig::parse(&nested),
            Err(ConfigError::UnknownField {
                field: "watch.bogus".into()
            })
        );
    }

    #[test]
    fn schema_version_missing_or_unsupported_is_rejected() {
        let missing = "mode = \"observe_only\"\n";
        assert_eq!(
            CoreConfig::parse(missing),
            Err(ConfigError::MissingSchemaVersion)
        );
        let future = VALID.replace("schema_version = 1", "schema_version = 2");
        assert_eq!(
            CoreConfig::parse(&future),
            Err(ConfigError::UnsupportedSchemaVersion {
                found: 2,
                supported: 1
            })
        );
        let negative = VALID.replace("schema_version = 1", "schema_version = -1");
        assert_eq!(
            CoreConfig::parse(&negative),
            Err(ConfigError::UnsupportedSchemaVersion {
                found: -1,
                supported: 1
            })
        );
    }

    #[test]
    fn policy_modes_are_rejected_not_defaulted() {
        let policy = VALID.replace("observe_only", "balanced_actions_enabled");
        assert_eq!(
            CoreConfig::parse(&policy),
            Err(ConfigError::UnsupportedMode {
                found: "balanced_actions_enabled".to_string(),
                supported: MODE_OBSERVE_ONLY
            })
        );
    }

    #[test]
    fn out_of_range_values_are_field_errors() {
        let low = VALID.replace("interval_ms = 2000", "interval_ms = 5");
        assert_eq!(
            CoreConfig::parse(&low),
            Err(ConfigError::OutOfRange {
                field: "watch.interval_ms".to_string(),
                reason: "5 not in 100..=3600000".to_string()
            })
        );
        let huge = VALID.replace("retention_ms = 900000", "retention_ms = 99999999999");
        assert!(matches!(
            CoreConfig::parse(&huge),
            Err(ConfigError::OutOfRange { field, .. }) if field == "watch.retention_ms"
        ));
    }

    #[test]
    fn malformed_toml_is_reported_without_partial_state() {
        let broken = "schema_version = 1\n[watch\n";
        assert!(matches!(
            CoreConfig::parse(broken),
            Err(ConfigError::Malformed(_))
        ));
    }

    #[test]
    fn minimal_document_uses_defaults_inside_watch() {
        let minimal = "schema_version = 1\n";
        let config = CoreConfig::parse(minimal).expect("minimal valid");
        assert_eq!(config.watch, WatchSettings::default());
        assert_eq!(config.mode, MODE_OBSERVE_ONLY);
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn agent_section_parses_pipe_name() {
        let text = r#"schema_version = 1
[agent]
pipe_name = "\\\\.\\pipe\\custom"
"#;
        let config = CoreConfig::parse(text).expect("valid");
        assert_eq!(config.agent.pipe_name.as_deref(), Some(r"\\.\pipe\custom"));
        // Absent section -> default (runtime-derived pipe name).
        let minimal = CoreConfig::parse("schema_version = 1\n").expect("valid");
        assert_eq!(minimal.agent, AgentSettings::default());
        assert!(minimal.agent.pipe_name.is_none());
        // Roundtrip keeps the field.
        let back = CoreConfig::parse(&config.to_toml().expect("ser")).expect("re-parse");
        assert_eq!(back, config);
    }

    #[test]
    fn agent_unknown_field_is_rejected_by_name() {
        let text = "schema_version = 1\n[agent]\nnope = 1\n";
        assert_eq!(
            CoreConfig::parse(text),
            Err(ConfigError::UnknownField {
                field: "agent.nope".to_string()
            })
        );
    }
}

#[cfg(test)]
mod cohort_tests {
    use super::*;
    use crate::cohort::HardwareCohort;

    #[test]
    fn cohort_section_parses_every_declaration() {
        for (text, expected) in [
            ("4gib", HardwareCohort::Cohort4GiB),
            ("8gib", HardwareCohort::Cohort8GiB),
            ("16gib", HardwareCohort::Cohort16GiB),
            ("none", HardwareCohort::NonCohort),
        ] {
            let doc = format!("schema_version = 1\n[cohort]\ndeclared = \"{text}\"\n");
            let config = CoreConfig::parse(&doc).expect("valid");
            assert_eq!(config.cohort.declared, expected, "declaration {text}");
            // Roundtrip keeps the declaration.
            let back = CoreConfig::parse(&config.to_toml().expect("ser")).expect("re-parse");
            assert_eq!(back, config);
        }
    }

    #[test]
    fn absent_cohort_section_defaults_to_none() {
        let config = CoreConfig::parse("schema_version = 1\n").expect("valid");
        assert_eq!(config.cohort, CohortSettings::default());
        assert_eq!(config.cohort.declared, HardwareCohort::NonCohort);
        // An empty [cohort] table is equally a default declaration.
        let empty = CoreConfig::parse("schema_version = 1\n[cohort]\n").expect("valid");
        assert_eq!(empty.cohort.declared, HardwareCohort::NonCohort);
    }

    #[test]
    fn unknown_cohort_declaration_is_a_field_error() {
        for bad in ["64gib", "16 GB", "", "bogus"] {
            let doc = format!("schema_version = 1\n[cohort]\ndeclared = \"{bad}\"\n");
            assert_eq!(
                CoreConfig::parse(&doc),
                Err(ConfigError::UnsupportedCohort {
                    found: bad.to_string()
                }),
                "declaration {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn cohort_unknown_field_is_rejected_by_name() {
        let text = "schema_version = 1\n[cohort]\ndeclared = \"none\"\nnominal = 8\n";
        assert_eq!(
            CoreConfig::parse(text),
            Err(ConfigError::UnknownField {
                field: "cohort.nominal".to_string()
            })
        );
    }

    /// The declaration is a label: a config that declares a cohort parses
    /// fine even though this process cannot verify hardware (verification is
    /// measurement-time, since core makes no OS calls).
    #[test]
    fn declaration_is_accept_without_machine_verification() {
        let text = "schema_version = 1\n[cohort]\ndeclared = \"8gib\"\n";
        let config = CoreConfig::parse(text).expect("valid");
        assert_eq!(config.cohort.declared, HardwareCohort::Cohort8GiB);
        assert_eq!(config.mode, MODE_OBSERVE_ONLY);
    }
}

#[cfg(test)]
mod profiles_tests {
    use super::*;

    #[test]
    fn every_preset_parses_and_defaults_to_balanced() {
        for name in PROFILES {
            let doc = format!("schema_version = 1\n[profiles]\nactive = \"{name}\"\n");
            let config = CoreConfig::parse(&doc).expect("valid");
            assert_eq!(config.profiles.active, *name, "preset {name}");
            // Roundtrip keeps the selection.
            let back = CoreConfig::parse(&config.to_toml().expect("ser")).expect("re-parse");
            assert_eq!(back, config);
        }
        // Absent section -> balanced (contract 14 §5: presets share defaults;
        // the selection is inert either way).
        let minimal = CoreConfig::parse("schema_version = 1\n").expect("valid");
        assert_eq!(minimal.profiles, ProfileSettings::default());
        assert_eq!(minimal.profiles.active, PROFILE_DEFAULT);
    }

    #[test]
    fn unknown_profile_is_a_field_error() {
        let doc = "schema_version = 1\n[profiles]\nactive = \"turbo\"\n";
        assert_eq!(
            CoreConfig::parse(doc),
            Err(ConfigError::UnsupportedProfile {
                found: "turbo".to_string()
            })
        );
        let empty = "schema_version = 1\n[profiles]\nactive = \"\"\n";
        assert!(matches!(
            CoreConfig::parse(empty),
            Err(ConfigError::UnsupportedProfile { .. })
        ));
    }

    #[test]
    fn profiles_unknown_field_is_rejected_by_name() {
        let text = "schema_version = 1\n[profiles]\nactive = \"focus\"\nbooster = 1\n";
        assert_eq!(
            CoreConfig::parse(text),
            Err(ConfigError::UnknownField {
                field: "profiles.booster".to_string()
            })
        );
    }

    /// Selecting a profile never changes the mode or any setting: the only
    /// observable difference is the stored label itself.
    #[test]
    fn profile_selection_is_inert() {
        let a = CoreConfig::parse("schema_version = 1\n").expect("valid");
        let b = CoreConfig::parse("schema_version = 1\n[profiles]\nactive = \"low_ram\"\n")
            .expect("valid");
        assert_eq!(a.watch, b.watch);
        assert_eq!(a.mode, b.mode);
        assert_eq!(a.exclusions, b.exclusions);
        assert_ne!(a.profiles, b.profiles);
    }
}

#[cfg(test)]
mod exclusions_tests {
    use super::*;

    #[test]
    fn absent_section_is_empty_and_roundtrips() {
        let config = CoreConfig::parse("schema_version = 1\n").expect("valid");
        assert!(config.exclusions.processes.is_empty());
        // An explicit empty list serializes away and stays equal.
        let empty = CoreConfig::parse("schema_version = 1\n[exclusions]\n").expect("valid");
        assert_eq!(empty.exclusions, ExclusionSettings::default());
    }

    #[test]
    fn exclusion_list_parses_and_roundtrips() {
        let text =
            "schema_version = 1\n[exclusions]\nprocesses = [\"obs64.exe\", \"Audacity.exe\"]\n";
        let config = CoreConfig::parse(text).expect("valid");
        assert_eq!(config.exclusions.processes, ["obs64.exe", "Audacity.exe"]);
        let back = CoreConfig::parse(&config.to_toml().expect("ser")).expect("re-parse");
        assert_eq!(back, config);
    }

    #[test]
    fn exclusion_entries_are_names_not_paths() {
        // TOML literal strings (single quotes) carry backslashes verbatim;
        // the newline case uses an escaped basic string.
        for doc in [
            "schema_version = 1\n[exclusions]\nprocesses = ['C:\\evil\\tool.exe']\n",
            "schema_version = 1\n[exclusions]\nprocesses = ['dir/tool.exe']\n",
            "schema_version = 1\n[exclusions]\nprocesses = ['']\n",
            "schema_version = 1\n[exclusions]\nprocesses = [\"a\\nb\"]\n",
        ] {
            assert!(
                matches!(
                    CoreConfig::parse(doc),
                    Err(ConfigError::OutOfRange { field, .. }) if field.starts_with("exclusions.processes")
                ),
                "{doc:?} must be rejected"
            );
        }
    }

    #[test]
    fn exclusion_count_is_bounded() {
        let entries: Vec<String> = (0..513).map(|i| format!("tool{i}.exe")).collect();
        let list = entries.join("\", \"");
        let doc = format!("schema_version = 1\n[exclusions]\nprocesses = [\"{list}\"]\n");
        assert!(matches!(
            CoreConfig::parse(&doc),
            Err(ConfigError::OutOfRange { field, reason })
                if field == "exclusions.processes" && reason.contains("513")
        ));
    }

    #[test]
    fn exclusions_unknown_field_is_rejected_by_name() {
        let text = "schema_version = 1\n[exclusions]\npaths = [\"x\"]\n";
        assert_eq!(
            CoreConfig::parse(text),
            Err(ConfigError::UnknownField {
                field: "exclusions.paths".to_string()
            })
        );
    }
}
