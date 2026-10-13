//! Raycast export migration and the deliberately small standalone JavaScript SDK adapter.
//!
//! Imports are previews first and produce disabled automation. This module never evaluates a
//! Raycast script while inspecting it; JavaScript only runs after its command is explicitly
//! enabled, through the shared bounded automation runner.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::automation::{
    AutomationError, AutomationRunner, DEFAULT_OUTPUT_LIMIT, ExecuteRequest, ScriptCommand,
};
use crate::quicklinks::{Quicklink, WebSearchTemplate, destination};
use crate::snippets::Snippet;
use crate::util::new_id;

/// Maximum accepted Raycast export size.
pub const MAX_RAYCAST_EXPORT_BYTES: usize = 10_000_000;
/// Maximum entries in one Raycast export.
pub const MAX_RAYCAST_EXPORT_ITEMS: usize = 10_000;
/// Maximum package manifest, script or standalone JavaScript source size.
pub const MAX_RAYCAST_SOURCE_BYTES: usize = 1_000_000;

const VALIDATION_HOME: &str = "/home/validation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RaycastImportKind {
    Snippet,
    Quicklink,
}

/// One item shown in the selective export preview. `issue` disables import for the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaycastImportItem {
    /// Stable within this preview, based on its source index.
    pub id: String,
    pub kind: RaycastImportKind,
    pub name: String,
    /// Snippet text or quicklink target.
    pub value: String,
    /// Quicklink keyword, normalized to lowercase. Snippet keywords are intentionally omitted.
    pub keyword: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExportRecord {
    name: String,
    text: Option<String>,
    link: Option<String>,
    url: Option<String>,
    keyword: Option<String>,
}

fn placeholder_pattern() -> Option<&'static regex::Regex> {
    static PATTERN: OnceLock<Option<regex::Regex>> = OnceLock::new();
    PATTERN
        .get_or_init(|| regex::Regex::new(r"\{[^{}]+\}").ok())
        .as_ref()
}

fn has_dynamic_placeholder(text: &str) -> bool {
    placeholder_pattern().is_some_and(|pattern| pattern.is_match(text))
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

/// Decode an individual Raycast Snippets or Quicklinks JSON export for review.
pub fn preview_export(data: &[u8]) -> Result<Vec<RaycastImportItem>, AutomationError> {
    if data.len() > MAX_RAYCAST_EXPORT_BYTES {
        return Err(AutomationError::Invalid("The export exceeds 10 MB.".into()));
    }
    let records: Vec<ExportRecord> = serde_json::from_slice(data).map_err(|_| {
        AutomationError::Invalid(
            "Choose the JSON from Raycast's Export Snippets or Export Quicklinks command. Encrypted .rayconfig backups are not supported.".into(),
        )
    })?;
    if records.len() > MAX_RAYCAST_EXPORT_ITEMS {
        return Err(AutomationError::Invalid(
            "The export exceeds 10,000 items.".into(),
        ));
    }

    Ok(records
        .into_iter()
        .enumerate()
        .map(|(index, record)| {
            let kind = if record.text.is_some() {
                RaycastImportKind::Snippet
            } else {
                RaycastImportKind::Quicklink
            };
            let value = record
                .text
                .clone()
                .or_else(|| record.link.clone())
                .or_else(|| record.url.clone())
                .unwrap_or_default();
            let name = record.name.trim().to_owned();
            let mut issue = None;
            if record.text.is_some() && (record.link.is_some() || record.url.is_some()) {
                issue = Some("Ambiguous item with both text and a link.".into());
            } else if kind == RaycastImportKind::Snippet {
                if has_dynamic_placeholder(&value) {
                    issue = Some(
                        "Dynamic placeholders require manual conversion before importing.".into(),
                    );
                } else if let Err(error) = Snippet::new(name.clone(), value.clone()).validate() {
                    issue = Some(error.to_string());
                }
            } else if name.is_empty() || destination(&value, "", VALIDATION_HOME).is_none() {
                issue = Some(
                    "Unsupported link. Use http(s), an absolute file path, or a {query} URL template.".into(),
                );
            }
            let keyword = if kind == RaycastImportKind::Quicklink {
                record
                    .keyword
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase()
            } else {
                String::new()
            };
            if kind == RaycastImportKind::Quicklink
                && keyword.chars().any(char::is_whitespace)
            {
                issue = Some("The quicklink keyword must be a single word.".into());
            }
            RaycastImportItem {
                id: index.to_string(),
                kind,
                name,
                value,
                keyword,
                issue,
            }
        })
        .collect())
}

/// Report why a valid preview item conflicts with current configuration.
pub fn import_conflict(
    item: &RaycastImportItem,
    snippets: &[Snippet],
    quicklinks: &[Quicklink],
    web_searches: &[WebSearchTemplate],
) -> Option<String> {
    if let Some(issue) = &item.issue {
        return Some(issue.clone());
    }
    match item.kind {
        RaycastImportKind::Snippet => snippets
            .iter()
            .any(|snippet| same_name(&snippet.name, &item.name) || snippet.template == item.value)
            .then(|| "An existing snippet has this name or text; it will be preserved.".into()),
        RaycastImportKind::Quicklink => {
            if quicklinks
                .iter()
                .any(|link| same_name(&link.name, &item.name) || link.target == item.value)
            {
                return Some(
                    "An existing quicklink has this name or target; it will be preserved.".into(),
                );
            }
            if !item.keyword.is_empty()
                && (quicklinks
                    .iter()
                    .any(|link| same_name(&link.keyword, &item.keyword))
                    || web_searches
                        .iter()
                        .any(|search| same_name(&search.keyword, &item.keyword)))
            {
                return Some(
                    "This keyword is already in use; the existing item will be preserved.".into(),
                );
            }
            None
        }
    }
}

/// Add selected preview rows. Keywords on imported snippets are never enabled. Existing items
/// are preserved and counted as skipped; invalid selected batches are rejected before mutation.
pub fn import_selected(
    items: &[RaycastImportItem],
    selected_ids: &BTreeSet<String>,
    snippets: &mut Vec<Snippet>,
    quicklinks: &mut Vec<Quicklink>,
    web_searches: &[WebSearchTemplate],
    home: &str,
) -> Result<(usize, usize), AutomationError> {
    let item_ids: BTreeSet<_> = items.iter().map(|item| item.id.as_str()).collect();
    if item_ids.len() != items.len() {
        return Err(AutomationError::Invalid(
            "Duplicate import item identifiers.".into(),
        ));
    }

    for item in items
        .iter()
        .filter(|item| selected_ids.contains(&item.id) && item.issue.is_none())
    {
        validate_import_item(item, home)?;
    }

    let mut next_snippets = snippets.clone();
    let mut next_quicklinks = quicklinks.clone();
    let mut imported = 0;
    let mut skipped = 0;
    for item in items.iter().filter(|item| selected_ids.contains(&item.id)) {
        if import_conflict(item, &next_snippets, &next_quicklinks, web_searches).is_some() {
            skipped += 1;
            continue;
        }
        match item.kind {
            RaycastImportKind::Snippet => {
                next_snippets.push(Snippet::new(item.name.trim(), item.value.clone()));
            }
            RaycastImportKind::Quicklink => next_quicklinks.push(Quicklink {
                id: new_id(),
                name: item.name.trim().to_owned(),
                target: item.value.clone(),
                keyword: item.keyword.clone(),
            }),
        }
        imported += 1;
    }
    *snippets = next_snippets;
    *quicklinks = next_quicklinks;
    Ok((imported, skipped))
}

fn validate_import_item(item: &RaycastImportItem, home: &str) -> Result<(), AutomationError> {
    let failure = |message: &str| AutomationError::Invalid(message.into());
    if item.name.trim().is_empty() {
        return Err(failure("A Raycast item needs a name."));
    }
    match item.kind {
        RaycastImportKind::Snippet => {
            if has_dynamic_placeholder(&item.value) {
                return Err(failure("Dynamic snippet placeholders require conversion."));
            }
            Snippet::new(item.name.trim(), item.value.clone())
                .validate()
                .map_err(|error| AutomationError::Invalid(error.to_string()))
        }
        RaycastImportKind::Quicklink => {
            if destination(&item.value, "", home).is_none()
                || item.keyword.chars().any(char::is_whitespace)
            {
                return Err(failure("Invalid quicklink in the selected batch."));
            }
            Ok(())
        }
    }
}

/// Parse actual `@raycast` comment metadata without evaluating the script. The resulting command
/// uses the existing bounded Script Commands runner and always begins disabled.
pub fn inspect_script(path: impl AsRef<Path>) -> Result<ScriptCommand, AutomationError> {
    let path = path.as_ref();
    let data = fs::read(path).map_err(|error| AutomationError::Invalid(error.to_string()))?;
    if data.len() > MAX_RAYCAST_SOURCE_BYTES {
        return Err(AutomationError::Invalid(
            "Choose a UTF-8 Raycast script under 1 MB.".into(),
        ));
    }
    let source = std::str::from_utf8(&data).map_err(|_| {
        AutomationError::Invalid("Choose a UTF-8 Raycast script under 1 MB.".into())
    })?;
    let path = path.canonicalize().map_err(|error| {
        AutomationError::Invalid(format!("Could not resolve the script path: {error}"))
    })?;
    inspect_script_source(source, &path)
}

fn inspect_script_source(source: &str, path: &Path) -> Result<ScriptCommand, AutomationError> {
    let path_string = path
        .to_str()
        .filter(|path| path.starts_with('/') && !path.contains('\0'))
        .ok_or_else(|| AutomationError::Invalid("Invalid interpreter or script path.".into()))?;
    let mut metadata = BTreeMap::<String, String>::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') && !trimmed.starts_with("//") {
            continue;
        }
        let Some((_, rest)) = trimmed.split_once("@raycast.") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        if metadata.contains_key(key) {
            return Err(AutomationError::Invalid(format!(
                "Duplicate Raycast metadata: {key}."
            )));
        }
        metadata.insert(key.to_owned(), value.trim().to_owned());
    }

    let title = metadata
        .get("title")
        .filter(|title| !title.is_empty())
        .ok_or_else(|| {
            AutomationError::Invalid("Expected @raycast.schemaVersion 1 and @raycast.title.".into())
        })?;
    if metadata.get("schemaVersion").map(String::as_str) != Some("1") {
        return Err(AutomationError::Invalid(
            "Expected @raycast.schemaVersion 1 and @raycast.title.".into(),
        ));
    }
    let mode = metadata.get("mode").map(String::as_str).unwrap_or("");
    if !["silent", "compact", "fullOutput"].contains(&mode) {
        return Err(AutomationError::Invalid(
            "Only silent, compact and fullOutput scripts are supported. Inline refresh is not available.".into(),
        ));
    }
    if let Some(key) = metadata.keys().find(|key| {
        key.starts_with("argument")
            || ["currentDirectoryPath", "refreshTime"].contains(&key.as_str())
    }) {
        return Err(AutomationError::Invalid(format!(
            "Raycast {key} metadata is not supported; adapt this script explicitly first."
        )));
    }
    if metadata.get("needsConfirmation").map(String::as_str) == Some("true") {
        return Err(AutomationError::Invalid(
            "Scripts requiring per-run confirmation are not supported.".into(),
        ));
    }
    if metadata
        .get("platform")
        .is_some_and(|platform| platform != "macos")
    {
        return Err(AutomationError::Invalid(
            "Only macOS scripts are supported.".into(),
        ));
    }

    let first_line = source.lines().next().unwrap_or_default();
    let shebang = first_line.strip_prefix("#!/").ok_or_else(|| {
        AutomationError::Invalid("The script must have an absolute interpreter shebang.".into())
    })?;
    let mut interpreter: Vec<String> = shebang.split_whitespace().map(str::to_owned).collect();
    if let Some(executable) = interpreter.first_mut() {
        executable.insert(0, '/');
    }
    let executable = interpreter
        .first()
        .ok_or_else(|| AutomationError::Invalid("Invalid interpreter or script path.".into()))?;
    if !executable.starts_with('/') || executable.contains('\0') {
        return Err(AutomationError::Invalid(
            "Invalid interpreter or script path.".into(),
        ));
    }
    let directory = path
        .parent()
        .and_then(Path::to_str)
        .ok_or_else(|| AutomationError::Invalid("Invalid interpreter or script path.".into()))?;
    let mut arguments = vec![
        "-c".into(),
        "cd -- \"$1\" && shift && exec \"$@\"".into(),
        "kebloop-raycast".into(),
        directory.into(),
    ];
    arguments.extend(interpreter);
    arguments.push(path_string.to_owned());

    let description = metadata
        .get("description")
        .map(String::as_str)
        .unwrap_or("Imported script; review before enabling.");
    let mut command = ScriptCommand::new(new_id(), title, "/bin/sh", arguments);
    command.description = format!("Raycast {mode} · {description}");
    command.enabled = false;
    command.validate()?;
    Ok(command)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaycastSdkPreference {
    pub name: String,
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub preference_type: String,
    pub required: Option<bool>,
    #[serde(rename = "default")]
    pub default_value: Option<String>,
}

/// Standalone compiled JavaScript no-view command using the Raycast SDK compatibility shim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaycastSdkCommand {
    pub id: String,
    pub title: String,
    pub source: String,
    pub runtime: String,
    #[serde(default)]
    pub preferences: Vec<RaycastSdkPreference>,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
}

impl RaycastSdkCommand {
    /// Inspect a package command, entry point and explicitly selected local runtime.
    pub fn inspect(
        manifest_path: impl AsRef<Path>,
        command_name: &str,
        entry_path: impl AsRef<Path>,
        runtime: impl AsRef<Path>,
    ) -> Result<Self, AutomationError> {
        let runtime = runtime.as_ref();
        if !is_supported_runtime(runtime) || !runtime.is_file() || !is_executable(runtime) {
            return Err(AutomationError::Invalid(
                "Choose an installed node or bun executable by absolute path.".into(),
            ));
        }
        let manifest_data = fs::read(manifest_path.as_ref()).map_err(|error| {
            AutomationError::Invalid(format!("Could not read the package manifest: {error}"))
        })?;
        if manifest_data.len() > MAX_RAYCAST_SOURCE_BYTES {
            return Err(AutomationError::Invalid(
                "The package manifest exceeds 1 MB.".into(),
            ));
        }
        let manifest: SdkManifest = serde_json::from_slice(&manifest_data).map_err(|_| {
            AutomationError::Invalid("Choose a valid Raycast package.json manifest.".into())
        })?;
        let definition = manifest
            .commands
            .iter()
            .find(|definition| definition.name == command_name)
            .ok_or_else(|| {
                AutomationError::Invalid("Select a manifest command whose mode is no-view. React view commands are unsupported.".into())
            })?;
        if definition.mode != "no-view" {
            return Err(AutomationError::Invalid(
                "Select a manifest command whose mode is no-view. React view commands are unsupported.".into(),
            ));
        }
        if definition
            .arguments
            .as_ref()
            .is_some_and(|args| !args.is_empty())
        {
            return Err(AutomationError::Invalid(
                "Command argument forms are unsupported in this runtime.".into(),
            ));
        }
        let mut preferences = manifest.preferences.unwrap_or_default();
        preferences.extend(definition.preferences.clone().unwrap_or_default());
        let preference_names: BTreeSet<_> =
            preferences.iter().map(|item| item.name.as_str()).collect();
        if preference_names.len() != preferences.len()
            || preferences.iter().any(|preference| {
                preference.name.is_empty()
                    || !["textfield", "textarea"].contains(&preference.preference_type.as_str())
            })
        {
            return Err(AutomationError::Invalid(
                "Only unique textfield and textarea preferences are supported. Password, dropdown, OAuth and other preference types are unsupported.".into(),
            ));
        }
        let source_path = entry_path.as_ref();
        if !["js", "mjs"].contains(
            &source_path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
        ) {
            return Err(AutomationError::Invalid(
                "Choose a compiled, standalone .js or .mjs entry. TypeScript and JSX require compilation outside Kebloop.".into(),
            ));
        }
        let source = fs::read(source_path).map_err(|error| {
            AutomationError::Invalid(format!("Could not read the JavaScript entry: {error}"))
        })?;
        if source.len() > MAX_RAYCAST_SOURCE_BYTES || std::str::from_utf8(&source).is_err() {
            return Err(AutomationError::Invalid(
                "Choose a UTF-8 JavaScript command under 1 MB.".into(),
            ));
        }
        let title = definition.title.trim();
        if title.is_empty() {
            return Err(AutomationError::Invalid(
                "The selected Raycast command needs a title.".into(),
            ));
        }
        let source = source_path
            .canonicalize()
            .map_err(|error| AutomationError::Invalid(error.to_string()))?;
        let runtime = runtime.to_str().ok_or_else(|| {
            AutomationError::Invalid("The runtime path is not valid UTF-8.".into())
        })?;
        Ok(Self {
            id: new_id(),
            title: title.to_owned(),
            source: source.to_string_lossy().into_owned(),
            runtime: runtime.to_owned(),
            preferences,
            values: BTreeMap::new(),
            enabled: false,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SdkManifest {
    commands: Vec<SdkManifestCommand>,
    preferences: Option<Vec<RaycastSdkPreference>>,
}

#[derive(Debug, Deserialize)]
struct SdkManifestCommand {
    name: String,
    title: String,
    mode: String,
    preferences: Option<Vec<RaycastSdkPreference>>,
    arguments: Option<Vec<Value>>,
}

fn is_supported_runtime(path: &Path) -> bool {
    path.is_absolute()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "node" || name == "bun")
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// One validated native effect requested by the JavaScript shim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaycastSdkEffect {
    #[serde(rename = "type")]
    pub effect_type: String,
    pub value: String,
}

/// Structured response from a standalone no-view command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaycastSdkResponse {
    pub effects: Vec<RaycastSdkEffect>,
    pub error: Option<String>,
}

/// Run an enabled command and validate its entire effect batch before returning it to the host.
/// The caller applies `copy`, `message` and `open` effects after this function succeeds.
pub fn execute_sdk(
    command: &RaycastSdkCommand,
    runner: &AutomationRunner,
) -> Result<RaycastSdkResponse, AutomationError> {
    if !command.enabled {
        return Err(AutomationError::NotEnabled("Raycast JavaScript command"));
    }
    let runtime = PathBuf::from(&command.runtime);
    let source = PathBuf::from(&command.source);
    if !is_supported_runtime(&runtime)
        || !runtime.is_file()
        || !is_executable(&runtime)
        || !source.is_absolute()
        || source
            .extension()
            .and_then(|item| item.to_str())
            .is_none_or(|ext| !["js", "mjs"].contains(&ext.to_ascii_lowercase().as_str()))
        || command.id.is_empty()
        || command.title.is_empty()
        || command.title.len() > 200
        || command.preferences.len() > 100
        || command.values.len() > 100
    {
        return Err(AutomationError::Invalid(
            "Invalid stored Raycast extension configuration. Reimport after reviewing the manifest and runtime.".into(),
        ));
    }
    let preference_names: BTreeSet<_> = command
        .preferences
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    if preference_names.len() != command.preferences.len()
        || command.preferences.iter().any(|preference| {
            preference.name.is_empty()
                || preference.name.len() > 200
                || !["textfield", "textarea"].contains(&preference.preference_type.as_str())
                || preference
                    .default_value
                    .as_ref()
                    .is_some_and(|value| value.len() > 10_000)
        })
        || command
            .values
            .iter()
            .any(|(name, value)| name.len() > 200 || value.len() > 10_000)
    {
        return Err(AutomationError::Invalid(
            "Invalid stored Raycast extension configuration. Reimport after reviewing the manifest and runtime.".into(),
        ));
    }
    if command.preferences.iter().any(|preference| {
        preference.required == Some(true)
            && command
                .values
                .get(&preference.name)
                .or(preference.default_value.as_ref())
                .is_none_or(String::is_empty)
    }) {
        return Err(AutomationError::Invalid(
            "Fill all required extension preferences before running.".into(),
        ));
    }
    let source_bytes = fs::read(&source).map_err(|error| {
        AutomationError::Invalid(format!("Could not read the JavaScript command: {error}"))
    })?;
    if source_bytes.len() > MAX_RAYCAST_SOURCE_BYTES || std::str::from_utf8(&source_bytes).is_err()
    {
        return Err(AutomationError::Invalid(
            "The command must be UTF-8 and under 1 MB.".into(),
        ));
    }

    let directory = PrivateDirectory::create()?;
    let package = directory.0.join("node_modules/@raycast/api");
    fs::create_dir_all(&package).map_err(file_error)?;
    fs::write(directory.0.join("command.mjs"), &source_bytes).map_err(file_error)?;
    fs::write(
        package.join("package.json"),
        r#"{"type":"module","exports":"./index.mjs"}"#,
    )
    .map_err(file_error)?;
    fs::write(package.join("index.mjs"), SDK_SHIM).map_err(file_error)?;
    fs::write(directory.0.join("host.mjs"), SDK_HOST).map_err(file_error)?;

    let preferences: BTreeMap<_, _> = command
        .preferences
        .iter()
        .map(|preference| {
            (
                preference.name.clone(),
                command
                    .values
                    .get(&preference.name)
                    .cloned()
                    .or_else(|| preference.default_value.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();
    let input = serde_json::to_vec(&preferences)
        .map_err(|error| AutomationError::Invalid(error.to_string()))?;
    let output = runner.execute(
        &ExecuteRequest::new(
            command.runtime.clone(),
            vec![directory.0.join("host.mjs").to_string_lossy().into_owned()],
        )
        .with_input(input)
        .with_timeout(Duration::from_secs(30))
        .with_output_limit(DEFAULT_OUTPUT_LIMIT),
    )?;
    let response: RaycastSdkResponse = serde_json::from_str(&output.stdout).map_err(|_| {
        AutomationError::Invalid(
            "The extension returned invalid output. Check that it writes no data to stdout.".into(),
        )
    })?;
    if let Some(error) = response.error {
        return Err(AutomationError::Invalid(error));
    }
    validate_sdk_effects(&response.effects)?;
    Ok(response)
}

fn validate_sdk_effects(effects: &[RaycastSdkEffect]) -> Result<(), AutomationError> {
    if effects.len() > 100
        || effects.iter().any(|effect| {
            !["copy", "message", "open"].contains(&effect.effect_type.as_str())
                || effect.value.len() > 100_000
        })
    {
        return Err(AutomationError::Invalid(
            "The extension returned invalid effects.".into(),
        ));
    }
    for effect in effects.iter().filter(|effect| effect.effect_type == "open") {
        let is_http = Url::parse(&effect.value).is_ok_and(|url| {
            ["http", "https"].contains(&url.scheme())
                && url.host_str().is_some_and(|host| !host.is_empty())
        });
        if !is_http {
            return Err(AutomationError::Invalid(
                "Extension open only supports HTTP(S) URLs.".into(),
            ));
        }
    }
    Ok(())
}

fn file_error(error: std::io::Error) -> AutomationError {
    AutomationError::Invalid(format!(
        "Could not prepare the compatibility runtime: {error}"
    ))
}

struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    fn create() -> Result<Self, AutomationError> {
        for _ in 0..4 {
            let directory = std::env::temp_dir().join(format!("kebloop-raycast-sdk-{}", new_id()));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => return Ok(Self(directory)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(file_error(error)),
            }
        }
        Err(AutomationError::Invalid(
            "Could not create a private compatibility runtime directory.".into(),
        ))
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SDK_SHIM: &str = r#"
const effects = globalThis.__kebloopEffects;
function text(value) { if (typeof value !== "string" && typeof value !== "number") throw new Error("Only plain text is supported"); return String(value); }
function emit(type, value) { if (effects.length >= 100 || value.length > 100000) throw new Error("Effect limit exceeded"); effects.push({ type, value }); }
export const Clipboard = Object.freeze({ copy: async (value, options) => { if (options && Object.keys(options).length) throw new Error("Clipboard options are unsupported"); emit("copy", text(value)); } });
export const Toast = Object.freeze({ Style: Object.freeze({ Success: "success", Failure: "failure", Animated: "animated" }) });
export async function showToast(options) { if (typeof options !== "object" || options.primaryAction || options.secondaryAction) throw new Error("Toast actions are unsupported"); emit("message", text(options.title) + (options.message ? ": " + text(options.message) : "")); return { hide: async () => { throw new Error("Toast mutation/hiding is unsupported"); } }; }
export async function showHUD(value) { emit("message", text(value)); }
export function getPreferenceValues() { return { ...globalThis.__kebloopPreferences }; }
export async function open(value, application) { if (application) throw new Error("Application selection is unsupported"); const url = new URL(text(value)); if (!["http:", "https:"].includes(url.protocol)) throw new Error("Only HTTP(S) URLs are supported"); emit("open", url.href); }
"#;

const SDK_HOST: &str = r#"
import { readFileSync } from "node:fs";
globalThis.__kebloopEffects = [];
globalThis.__kebloopPreferences = JSON.parse(readFileSync(0, "utf8"));
console.log = console.info = (...args) => console.error(...args);
try {
  const command = await import("./command.mjs");
  if (typeof command.default !== "function") throw new Error("The no-view command must export a default function");
  await command.default({ arguments: {}, launchContext: undefined });
  process.stdout.write(JSON.stringify({ effects: globalThis.__kebloopEffects, error: null }));
} catch (error) {
  process.stdout.write(JSON.stringify({ effects: [], error: "Unsupported API or extension failure: " + error.message }));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::AutomationRunner;
    use std::fs;

    #[test]
    fn export_preview_rejects_dynamic_and_ambiguous_records() {
        let data = br#"[
            {"name":"Hello","text":"Hi {name}"},
            {"name":"Ambiguous","text":"body","url":"https://example.com"},
            {"name":"Search","url":"https://example.com/?q={query}","keyword":"go"},
            {"name":"Bad","url":"javascript:alert(1)"}
        ]"#;
        let preview = preview_export(data).expect("valid JSON export");
        assert_eq!(preview.len(), 4);
        assert_eq!(
            preview[0].issue.as_deref(),
            Some("Dynamic placeholders require manual conversion before importing.")
        );
        assert_eq!(
            preview[1].issue.as_deref(),
            Some("Ambiguous item with both text and a link.")
        );
        assert_eq!(preview[2].kind, RaycastImportKind::Quicklink);
        assert_eq!(preview[2].keyword, "go");
        assert!(preview[2].issue.is_none());
        assert!(preview[3].issue.is_some());
    }

    #[test]
    fn selected_export_import_is_additive_and_snippet_keywords_stay_off() {
        let preview = preview_export(
            br#"[{"name":"Reply","text":"Hello"},{"name":"Docs","link":"https://example.com","keyword":"DOCS"}]"#,
        )
        .expect("export preview");
        let selected = BTreeSet::from(["0".to_owned(), "1".to_owned()]);
        let mut snippets = vec![Snippet::new("Existing", "old")];
        let mut links = Vec::new();
        let (imported, skipped) = import_selected(
            &preview,
            &selected,
            &mut snippets,
            &mut links,
            &[],
            "/home/test",
        )
        .expect("import selected rows");
        assert_eq!((imported, skipped), (2, 0));
        assert_eq!(snippets[1].keyword, None);
        assert_eq!(links[0].keyword, "docs");

        let duplicate =
            preview_export(br#"[{"name":"reply","text":"Hello"}]"#).expect("duplicate preview");
        let selected = BTreeSet::from(["0".to_owned()]);
        let (imported, skipped) = import_selected(
            &duplicate,
            &selected,
            &mut snippets,
            &mut links,
            &[],
            "/home/test",
        )
        .expect("duplicate should be skipped");
        assert_eq!((imported, skipped), (0, 1));
        assert_eq!(snippets.len(), 2);
    }

    #[test]
    fn script_import_keeps_original_interpreter_and_starts_disabled() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("hello.sh");
        fs::write(
            &path,
            "#!/bin/bash -e\n# @raycast.schemaVersion 1\n# @raycast.title Hello\n# @raycast.mode fullOutput\necho hi\n",
        )
        .expect("script write");
        let command = inspect_script(&path).expect("script metadata");
        assert_eq!(command.title, "Hello");
        assert!(!command.enabled);
        assert_eq!(command.executable, "/bin/sh");
        assert_eq!(&command.arguments[4..6], ["/bin/bash", "-e"]);
        assert_eq!(
            command.arguments.last().map(String::as_str),
            Some(path.canonicalize().unwrap().to_str().unwrap())
        );

        fs::write(
            &path,
            "#!/bin/bash\n# @raycast.schemaVersion 1\n# @raycast.title Bad\n# @raycast.mode inline\n",
        )
        .expect("unsupported script write");
        assert!(inspect_script(&path).is_err());
    }

    fn sdk_files(directory: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let manifest = directory.join("package.json");
        let entry = directory.join("command.mjs");
        let runtime = directory.join("node");
        fs::copy("/bin/sh", &runtime).expect("runtime stub");
        fs::write(
            &manifest,
            r#"{"commands":[{"name":"hello","title":"Hello","mode":"no-view","preferences":[{"name":"recipient","title":"Recipient","type":"textfield","required":true}]}]}"#,
        )
        .expect("manifest write");
        fs::write(
            &entry,
            "import { Clipboard, getPreferenceValues, showHUD } from '@raycast/api'; export default async () => { const value = getPreferenceValues().recipient; await Clipboard.copy(value); await showHUD('Done'); };",
        )
        .expect("entry write");
        (manifest, entry, runtime)
    }

    #[test]
    fn sdk_import_rejects_view_commands_and_requires_text_preferences() {
        let directory = tempfile::tempdir().expect("temp directory");
        let (manifest, entry, runtime) = sdk_files(directory.path());
        let command = RaycastSdkCommand::inspect(&manifest, "hello", &entry, &runtime)
            .expect("no-view command");
        assert!(!command.enabled);
        assert_eq!(command.preferences[0].name, "recipient");

        fs::write(
            &manifest,
            r#"{"commands":[{"name":"hello","title":"Hello","mode":"view"}]}"#,
        )
        .expect("view manifest");
        assert!(RaycastSdkCommand::inspect(&manifest, "hello", &entry, &runtime).is_err());
    }

    #[test]
    fn sdk_effect_validation_rejects_entire_batch_if_open_is_unsupported() {
        let effects = vec![
            RaycastSdkEffect {
                effect_type: "copy".into(),
                value: "would copy".into(),
            },
            RaycastSdkEffect {
                effect_type: "open".into(),
                value: "file:///etc/passwd".into(),
            },
        ];
        assert!(validate_sdk_effects(&effects).is_err());
    }

    #[test]
    fn sdk_runs_through_shared_runner_and_returns_effects() {
        let runtime = Path::new("/usr/bin/node");
        if !runtime.is_file() {
            return;
        }
        let directory = tempfile::tempdir().expect("temp directory");
        let (manifest, entry, _) = sdk_files(directory.path());
        let mut command =
            RaycastSdkCommand::inspect(manifest, "hello", entry, runtime).expect("inspect");
        assert!(execute_sdk(&command, &AutomationRunner::new()).is_err());
        command.values.insert("recipient".into(), "Kebloop".into());
        command.enabled = true;
        let response = execute_sdk(&command, &AutomationRunner::new()).expect("execute shim");
        assert_eq!(response.effects.len(), 2);
        assert_eq!(response.effects[0].effect_type, "copy");
        assert_eq!(response.effects[0].value, "Kebloop");
        assert_eq!(response.effects[1].effect_type, "message");
    }
}
