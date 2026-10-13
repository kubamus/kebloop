//! Extension manifests (v1) and the JSON stdin/stdout protocol.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{AutomationError, AutomationRunner, DEFAULT_OUTPUT_LIMIT, ExecuteRequest};

/// Protocol timeout per invocation.
pub const EXTENSION_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum results in a response.
pub const MAX_RESULTS: usize = 100;
/// Maximum actions per result.
pub const MAX_ACTIONS: usize = 20;

/// A user-visible extension preference with its default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPreference {
    /// Preference key.
    pub id: String,
    /// Label.
    pub title: String,
    /// Default value.
    #[serde(default)]
    pub default_value: String,
}

/// Extension manifest, API version 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionManifest {
    /// Stable identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Must be 1.
    pub api_version: i64,
    /// Absolute executable path (runtime).
    pub executable: String,
    /// Literal arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Declared preferences.
    #[serde(default)]
    pub preferences: Vec<ExtensionPreference>,
    /// Enabled after review (default false).
    #[serde(default)]
    pub enabled: bool,
}

impl ExtensionManifest {
    /// Launcher command identifier (`extension:<id>`).
    pub fn command_id(&self) -> String {
        format!("extension:{}", self.id)
    }

    /// Check manifest rules.
    pub fn validate(&self) -> Result<(), AutomationError> {
        if self.api_version != 1 {
            return Err(AutomationError::Invalid(
                "Unsupported extension API version. Kebloop supports version 1.".into(),
            ));
        }
        let unique: BTreeSet<_> = self.preferences.iter().map(|p| &p.id).collect();
        let ok = !self.id.is_empty()
            && !self.name.is_empty()
            && self.executable.starts_with('/')
            && !self.executable.contains('\0')
            && self.arguments.iter().all(|a| !a.contains('\0'))
            && unique.len() == self.preferences.len();
        if ok {
            Ok(())
        } else {
            Err(AutomationError::Invalid(
                "Invalid extension manifest. Check its identifier, executable and preferences."
                    .into(),
            ))
        }
    }

    /// Effective preference values: saved values for declared keys, defaults otherwise.
    pub fn effective_preferences(
        &self,
        saved: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        self.preferences
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    saved
                        .get(&p.id)
                        .cloned()
                        .unwrap_or_else(|| p.default_value.clone()),
                )
            })
            .collect()
    }
}

/// Install or replace a manifest; installed manifests start disabled.
pub fn install_extension(
    manifests: &mut Vec<ExtensionManifest>,
    mut manifest: ExtensionManifest,
) -> Result<(), AutomationError> {
    manifest.validate()?;
    manifest.enabled = false;
    match manifests.iter_mut().find(|m| m.id == manifest.id) {
        Some(existing) => *existing = manifest,
        None => manifests.push(manifest),
    }
    Ok(())
}

/// Enable or disable an extension; returns whether it exists.
pub fn set_extension_enabled(manifests: &mut [ExtensionManifest], id: &str, enabled: bool) -> bool {
    match manifests.iter_mut().find(|m| m.id == id) {
        Some(manifest) => {
            manifest.enabled = enabled;
            true
        }
        None => false,
    }
}

/// Remove an extension and its saved preferences.
pub fn remove_extension(
    manifests: &mut Vec<ExtensionManifest>,
    preferences: &mut BTreeMap<String, BTreeMap<String, String>>,
    id: &str,
) {
    manifests.retain(|m| m.id != id);
    preferences.remove(id);
}

/// Save a preference value if the extension declares `key`; returns whether it was saved.
pub fn set_extension_preference(
    manifests: &[ExtensionManifest],
    saved: &mut BTreeMap<String, BTreeMap<String, String>>,
    extension_id: &str,
    key: &str,
    value: &str,
) -> bool {
    let declared = manifests
        .iter()
        .find(|m| m.id == extension_id)
        .is_some_and(|m| m.preferences.iter().any(|p| p.id == key));
    if declared {
        saved
            .entry(extension_id.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
    }
    declared
}

/// Drop manifests that no longer validate (used when loading stored data).
pub fn retain_valid_extensions(manifests: &mut Vec<ExtensionManifest>) {
    manifests.retain(|m| m.validate().is_ok());
}

/// An action offered on a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionAction {
    /// Action identifier, unique within its result.
    pub id: String,
    /// Label.
    pub title: String,
}

/// One result row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionResult {
    /// Result identifier, unique within a response.
    pub id: String,
    /// Title.
    pub title: String,
    /// Optional subtitle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Optional detail text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Actions (at most 20).
    #[serde(default)]
    pub actions: Vec<ExtensionAction>,
}

/// Request written to the extension's stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRequest {
    /// Always 1.
    pub api_version: i64,
    /// `search` or `action`.
    pub operation: String,
    /// Query text (empty for actions).
    pub query: String,
    /// Selected result (actions only).
    #[serde(default, rename = "resultID", skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// Selected action (actions only).
    #[serde(default, rename = "actionID", skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// Declared preferences only.
    #[serde(default)]
    pub preferences: BTreeMap<String, String>,
}

impl ExtensionRequest {
    /// Search request.
    pub fn search(query: &str, preferences: BTreeMap<String, String>) -> Self {
        Self {
            api_version: 1,
            operation: "search".into(),
            query: query.into(),
            result_id: None,
            action_id: None,
            preferences,
        }
    }

    /// Action request.
    pub fn action(result_id: &str, action_id: &str, preferences: BTreeMap<String, String>) -> Self {
        Self {
            api_version: 1,
            operation: "action".into(),
            query: String::new(),
            result_id: Some(result_id.into()),
            action_id: Some(action_id.into()),
            preferences,
        }
    }
}

/// Response read from the extension's stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResponse {
    /// Must be 1.
    pub api_version: i64,
    /// Results (at most 100).
    pub results: Vec<ExtensionResult>,
    /// Optional status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ExtensionResponse {
    /// Enforce version, size and uniqueness rules.
    pub fn validate(&self) -> Result<(), AutomationError> {
        if self.api_version != 1 {
            return Err(AutomationError::Invalid(
                "The extension returned an incompatible API version.".into(),
            ));
        }
        let result_ids: BTreeSet<_> = self.results.iter().map(|r| &r.id).collect();
        let ok = self.results.len() <= MAX_RESULTS
            && result_ids.len() == self.results.len()
            && self.results.iter().all(|r| {
                let action_ids: BTreeSet<_> = r.actions.iter().map(|a| &a.id).collect();
                !r.id.is_empty()
                    && !r.title.is_empty()
                    && r.actions.len() <= MAX_ACTIONS
                    && action_ids.len() == r.actions.len()
                    && r.actions
                        .iter()
                        .all(|a| !a.id.is_empty() && !a.title.is_empty())
            });
        if ok {
            Ok(())
        } else {
            Err(AutomationError::Invalid(
                "The extension returned invalid results or actions.".into(),
            ))
        }
    }
}

/// Run one protocol round trip (blocking; use a worker thread). Validates the response.
pub fn invoke_extension(
    manifest: &ExtensionManifest,
    request: &ExtensionRequest,
    runner: &AutomationRunner,
) -> Result<ExtensionResponse, AutomationError> {
    if !manifest.enabled {
        return Err(AutomationError::NotEnabled("extension"));
    }
    let input = serde_json::to_vec(request).map_err(|e| {
        AutomationError::Invalid(format!("Could not encode the extension request: {e}"))
    })?;
    let output = runner.execute(
        &ExecuteRequest::new(manifest.executable.clone(), manifest.arguments.clone())
            .with_input(input)
            .with_timeout(EXTENSION_TIMEOUT)
            .with_output_limit(DEFAULT_OUTPUT_LIMIT),
    )?;
    let response: ExtensionResponse = serde_json::from_str(&output.stdout).map_err(|_| {
        AutomationError::Invalid(
            "Extension returned malformed JSON. Check the version 1 response format.".into(),
        )
    })?;
    response.validate()?;
    Ok(response)
}

/// Tracks the last response of the active extension so only actions it offered can be invoked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionSession {
    active_id: Option<String>,
    results: Vec<ExtensionResult>,
    message: Option<String>,
}

impl ExtensionSession {
    /// Empty session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extension whose results are displayed.
    pub fn active_id(&self) -> Option<&str> {
        self.active_id.as_deref()
    }

    /// Results of the last successful response.
    pub fn results(&self) -> &[ExtensionResult] {
        &self.results
    }

    /// Message of the last successful response.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Forget results, message and the active extension (disable, remove, replace, reload).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Whether `action_id` of `result_id` was offered by the last response of `extension_id`.
    pub fn can_perform(&self, extension_id: &str, result_id: &str, action_id: &str) -> bool {
        self.active_id.as_deref() == Some(extension_id)
            && self
                .results
                .iter()
                .any(|r| r.id == result_id && r.actions.iter().any(|a| a.id == action_id))
    }

    /// Mark a search as started: clears previous results.
    pub fn begin_search(&mut self, extension_id: &str) {
        self.active_id = Some(extension_id.to_string());
        self.results.clear();
        self.message = None;
    }

    /// Apply the outcome of a request. Failures clear the results and are returned.
    pub fn finish(
        &mut self,
        outcome: Result<ExtensionResponse, AutomationError>,
    ) -> Result<(), AutomationError> {
        match outcome {
            Ok(response) => {
                self.results = response.results;
                self.message = response.message;
                Ok(())
            }
            Err(error) => {
                self.results.clear();
                self.message = None;
                Err(error)
            }
        }
    }

    /// Blocking search: `begin_search`, run, `finish`.
    pub fn search(
        &mut self,
        manifest: &ExtensionManifest,
        query: &str,
        saved_preferences: &BTreeMap<String, String>,
        runner: &AutomationRunner,
    ) -> Result<(), AutomationError> {
        if !manifest.enabled {
            return Err(AutomationError::NotEnabled("extension"));
        }
        self.begin_search(&manifest.id);
        let request =
            ExtensionRequest::search(query, manifest.effective_preferences(saved_preferences));
        self.finish(invoke_extension(manifest, &request, runner))
    }

    /// Blocking action. Fails with [`AutomationError::ActionUnavailable`] (leaving state intact)
    /// unless the action was part of the last response.
    pub fn perform(
        &mut self,
        manifest: &ExtensionManifest,
        result_id: &str,
        action_id: &str,
        saved_preferences: &BTreeMap<String, String>,
        runner: &AutomationRunner,
    ) -> Result<(), AutomationError> {
        if !self.can_perform(&manifest.id, result_id, action_id) {
            return Err(AutomationError::ActionUnavailable);
        }
        let request = ExtensionRequest::action(
            result_id,
            action_id,
            manifest.effective_preferences(saved_preferences),
        );
        self.message = None;
        self.finish(invoke_extension(manifest, &request, runner))
    }
}
