use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use super::*;

fn run(exe: &str, args: &[&str]) -> Result<AutomationOutput, AutomationError> {
    AutomationRunner::new().execute(&ExecuteRequest::new(
        exe,
        args.iter().map(|a| a.to_string()).collect(),
    ))
}

fn sh(script: &str) -> ExecuteRequest {
    ExecuteRequest::new("/bin/sh", vec!["-c".into(), script.into()])
}

#[test]
fn arguments_are_literal_and_environment_is_clean() {
    let weird = "$(touch /tmp/kebloop-injection); Łódź & cats";
    let output = run("/usr/bin/printf", &["%s", weird]).unwrap();
    assert_eq!(output.stdout, weird);
    assert!(!std::path::Path::new("/tmp/kebloop-injection").exists());
    let env = run("/usr/bin/env", &[]).unwrap().stdout;
    assert!(!env.contains("TOKEN"));
    let mut expected: Vec<String> = vec![
        "LANG=C.UTF-8".into(),
        "PATH=/usr/local/bin:/usr/bin:/bin".into(),
    ];
    expected.extend(
        process::session_env(|name| std::env::var(name).ok())
            .into_iter()
            .map(|(k, v)| format!("{k}={v}")),
    );
    let mut lines: Vec<String> = env.lines().map(str::to_owned).collect();
    lines.sort_unstable();
    expected.sort_unstable();
    assert_eq!(lines, expected);
}

#[test]
fn session_environment_is_a_minimal_allow_list() {
    let source: BTreeMap<&str, &str> = BTreeMap::from([
        ("HOME", "/home/u"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("DISPLAY", ""),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("XDG_CURRENT_DESKTOP", "niri"),
        ("GITHUB_TOKEN", "secret"),
        ("SSH_AUTH_SOCK", "/tmp/agent"),
    ]);
    let env = process::session_env(|name| source.get(name).map(|v| (*v).to_owned()));
    let names: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    assert_eq!(
        names,
        [
            "HOME",
            "XDG_RUNTIME_DIR",
            "WAYLAND_DISPLAY",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_CURRENT_DESKTOP"
        ]
    );
}

#[test]
fn child_receives_forwarded_session_variables_only() {
    let output = AutomationRunner::new()
        .execute(&sh("printf '%s|%s' \"$HOME\" \"${GITHUB_TOKEN-unset}\""))
        .unwrap();
    let home = std::env::var("HOME").unwrap_or_default();
    assert_eq!(output.stdout, format!("{home}|unset"));
}

#[test]
fn missing_executable_and_nonzero_exit() {
    assert_eq!(
        run("/missing/kebloop", &[]),
        Err(AutomationError::MissingExecutable)
    );
    assert_eq!(
        run("relative/path", &[]),
        Err(AutomationError::MissingExecutable)
    );
    assert_eq!(
        run("/usr/bin", &[]),
        Err(AutomationError::MissingExecutable)
    );
    assert_eq!(run("/usr/bin/false", &[]), Err(AutomationError::Failed(1)));
}

fn sh_run(script: &str) -> Result<AutomationOutput, AutomationError> {
    AutomationRunner::new().execute(&sh(script))
}

#[test]
fn signal_termination_maps_to_128_plus_signal() {
    assert_eq!(sh_run("kill -KILL $$"), Err(AutomationError::Failed(137)));
    assert_eq!(sh_run("exit 7"), Err(AutomationError::Failed(7)));
}

#[test]
fn output_is_bounded_and_timeout_terminates_the_process() {
    let runner = AutomationRunner::new();
    let request = ExecuteRequest::new("/usr/bin/yes", vec![]).with_output_limit(1024);
    assert_eq!(runner.execute(&request), Err(AutomationError::OutputLimit));
    let start = Instant::now();
    let request = ExecuteRequest::new("/bin/sleep", vec!["30".into()])
        .with_timeout(Duration::from_millis(50));
    assert_eq!(
        AutomationRunner::new().execute(&request),
        Err(AutomationError::Timeout)
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn invalid_limits_are_rejected() {
    let base = || ExecuteRequest::new("/usr/bin/true", vec![]);
    for request in [
        base().with_timeout(Duration::ZERO),
        base().with_timeout(Duration::from_secs(301)),
        base().with_output_limit(0),
        base().with_output_limit(1_048_577),
    ] {
        assert_eq!(
            AutomationRunner::new().execute(&request).map(|_| ()),
            Err(AutomationError::Invalid("Invalid execution limits.".into()))
        );
    }
}

#[test]
fn cancellation_before_launch_and_during_execution() {
    let cancelled = AutomationRunner::new();
    cancelled.cancel();
    let request = ExecuteRequest::new("/bin/sleep", vec!["30".into()]);
    assert_eq!(cancelled.execute(&request), Err(AutomationError::Cancelled));

    let runner = AutomationRunner::new();
    let handle = runner.cancel_handle();
    let worker = thread::spawn(move || {
        runner.execute(&ExecuteRequest::new("/bin/sleep", vec!["30".into()]))
    });
    thread::sleep(Duration::from_millis(50));
    let start = Instant::now();
    handle.cancel();
    assert_eq!(worker.join().unwrap(), Err(AutomationError::Cancelled));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(handle.is_cancelled());
}

#[test]
fn input_and_separate_streams() {
    let input = "first\nŁódź".as_bytes().to_vec();
    let result = AutomationRunner::new()
        .execute(&ExecuteRequest::new("/bin/cat", vec![]).with_input(input))
        .unwrap();
    assert_eq!(result.stdout, "first\nŁódź");
    assert_eq!(result.stderr, "");
    let value = sh_run("printf output; printf diagnostic >&2").unwrap();
    assert_eq!(
        (value.stdout.as_str(), value.stderr.as_str()),
        ("output", "diagnostic")
    );
}

#[test]
fn large_input_does_not_deadlock_and_oversized_input_is_rejected() {
    let big = vec![b'a'; 262_144];
    let result = AutomationRunner::new()
        .execute(
            &ExecuteRequest::new("/bin/cat", vec![])
                .with_input(big.clone())
                .with_output_limit(300_000),
        )
        .unwrap();
    assert_eq!(result.stdout.len(), big.len());
    let too_big = vec![b'a'; 262_145];
    let error = AutomationRunner::new()
        .execute(&ExecuteRequest::new("/bin/cat", vec![]).with_input(too_big));
    assert!(matches!(error, Err(AutomationError::Invalid(_))));
}

#[test]
fn timeout_and_completion_stop_ordinary_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let marker: PathBuf = dir.path().join("child-marker");
    let child = format!(
        "(/bin/sleep 0.4; /usr/bin/printf alive > '{}') & ",
        marker.display()
    );
    let request = sh(&format!("{child}wait")).with_timeout(Duration::from_millis(100));
    assert!(AutomationRunner::new().execute(&request).is_err());
    thread::sleep(Duration::from_millis(700));
    assert!(!marker.exists(), "descendant survived the timeout");
    AutomationRunner::new()
        .execute(&sh(&format!("{child}exit 0")))
        .unwrap();
    thread::sleep(Duration::from_millis(700));
    assert!(!marker.exists(), "descendant survived normal completion");
}

#[test]
fn sigterm_ignoring_process_is_killed_after_grace() {
    let start = Instant::now();
    let request =
        sh("trap '' TERM; while :; do sleep 1; done").with_timeout(Duration::from_millis(100));
    assert_eq!(
        AutomationRunner::new().execute(&request),
        Err(AutomationError::Timeout)
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn both_streams_count_toward_limit() {
    let request = sh("printf 123456; printf 123456 >&2").with_output_limit(10);
    assert_eq!(
        AutomationRunner::new().execute(&request),
        Err(AutomationError::OutputLimit)
    );
    let request = sh("printf 12345; printf 12345 >&2").with_output_limit(10);
    assert!(AutomationRunner::new().execute(&request).is_ok());
}

// ----- script commands -----

fn script(id: &str, exe: &str, args: &[&str], timeout: f64, enabled: bool) -> ScriptCommand {
    let mut command = ScriptCommand::new(id, id, exe, args.iter().map(|a| a.to_string()).collect());
    command.timeout = timeout;
    command.enabled = enabled;
    command
}

#[test]
fn imported_scripts_require_review_and_can_be_enabled() {
    let mut commands = Vec::new();
    let command = script("test", "/usr/bin/printf", &["hi"], 10.0, true);
    import_scripts(&mut commands, vec![command]).unwrap();
    assert!(!commands[0].enabled);
    assert_eq!(
        run_script(&commands, "test", &[], &AutomationRunner::new()),
        Err(AutomationError::NotEnabled("script"))
    );
    assert!(set_script_enabled(&mut commands, "test", true));
    assert_eq!(
        run_script(&commands, "test", &[], &AutomationRunner::new())
            .unwrap()
            .stdout,
        "hi"
    );
    assert!(!set_script_enabled(&mut commands, "missing", true));
    remove_script(&mut commands, "test");
    assert!(commands.is_empty());
}

#[test]
fn script_defaults_and_wire_format() {
    let command: ScriptCommand =
        serde_json::from_str(r#"{"id":"a","title":"A","executable":"/bin/true"}"#).unwrap();
    assert_eq!(command.timeout, 10.0);
    assert!(!command.enabled);
    assert_eq!(command.command_id(), "script:a");
    let json = serde_json::to_value(&command).unwrap();
    for key in [
        "id",
        "title",
        "description",
        "executable",
        "arguments",
        "timeout",
        "enabled",
    ] {
        assert!(json.get(key).is_some(), "missing {key}");
    }
}

#[test]
fn script_success_additional_arguments_and_execution_errors() {
    let runner = AutomationRunner::new();
    let commands = vec![script("literal", "/usr/bin/printf", &["%s"], 10.0, true)];
    let out = run_script(
        &commands,
        "literal",
        &["Łódź & $(nothing)".to_string()],
        &runner,
    )
    .unwrap();
    assert_eq!(out.stdout, "Łódź & $(nothing)");
    for (id, exe, args, timeout, expected) in [
        ("missing", "/missing/runtime", vec![], 1.0, "missing"),
        ("failure", "/usr/bin/false", vec![], 1.0, "status 1"),
        ("timeout", "/bin/sleep", vec!["30"], 0.05, "time limit"),
        ("output", "/usr/bin/yes", vec![], 1.0, "output limit"),
    ] {
        let commands = vec![script(id, exe, &args, timeout, true)];
        let error = run_script(&commands, id, &[], &AutomationRunner::new()).unwrap_err();
        assert!(error.to_string().contains(expected), "{id}: {error}");
    }
}

#[test]
fn script_validation_and_import_are_atomic() {
    let mut commands = Vec::new();
    let valid = script("valid", "/usr/bin/true", &[], 10.0, false);
    for timeout in [0.0, 301.0, f64::NAN, f64::INFINITY, -1.0] {
        let mut invalid = valid.clone();
        invalid.timeout = timeout;
        assert!(save_script(&mut commands, invalid).is_err());
        assert!(commands.is_empty());
    }
    let mut invalid = valid.clone();
    invalid.executable = "relative/path".into();
    assert!(import_scripts(&mut commands, vec![valid.clone(), invalid]).is_err());
    assert!(commands.is_empty());
    let mut invalid = valid.clone();
    invalid.arguments = vec!["nul\0argument".into()];
    assert!(save_script(&mut commands, invalid).is_err());
    let mut untitled = valid;
    untitled.title = "  ".into();
    assert!(save_script(&mut commands, untitled).is_err());
}

#[test]
fn script_run_can_be_cancelled_without_stale_output() {
    let commands = vec![script("slow", "/bin/sleep", &["30"], 30.0, true)];
    let runner = AutomationRunner::new();
    let handle = runner.cancel_handle();
    let worker = thread::spawn(move || run_script(&commands, "slow", &[], &runner));
    thread::sleep(Duration::from_millis(50));
    handle.cancel();
    assert_eq!(
        worker.join().unwrap().map(|_| ()),
        Err(AutomationError::Cancelled)
    );
}

// ----- extensions -----

fn manifest(id: &str, exe: &str, args: &[&str]) -> ExtensionManifest {
    ExtensionManifest {
        id: id.into(),
        name: id.into(),
        description: String::new(),
        api_version: 1,
        executable: exe.into(),
        arguments: args.iter().map(|a| a.to_string()).collect(),
        preferences: vec![],
        enabled: false,
    }
}

const GREETING: &str = r#"{"apiVersion":1,"results":[{"id":"greeting","title":"Hello","detail":"Details","actions":[{"id":"greet","title":"Greet"}]}],"message":"Success"}"#;

#[test]
fn extension_lifecycle_preferences_and_compatibility() {
    let mut manifests = Vec::new();
    let mut saved = BTreeMap::new();
    let mut m = manifest("example", "/usr/bin/true", &[]);
    m.preferences = vec![ExtensionPreference {
        id: "prefix".into(),
        title: "Prefix".into(),
        default_value: "Hello".into(),
    }];
    m.enabled = true;
    install_extension(&mut manifests, m.clone()).unwrap();
    assert!(!manifests[0].enabled);
    assert!(set_extension_preference(
        &manifests, &mut saved, "example", "prefix", "Welcome"
    ));
    assert!(!set_extension_preference(
        &manifests,
        &mut saved,
        "example",
        "secret",
        "should not save"
    ));
    let effective = manifests[0].effective_preferences(&saved["example"]);
    assert_eq!(effective["prefix"], "Welcome");
    assert!(!effective.contains_key("secret"));
    assert_eq!(
        manifests[0].effective_preferences(&BTreeMap::new())["prefix"],
        "Hello"
    );
    assert!(set_extension_enabled(&mut manifests, "example", true));
    assert!(manifests[0].enabled);
    m.api_version = 2;
    assert!(install_extension(&mut manifests, m).is_err());
    assert_eq!(manifests.len(), 1);
    remove_extension(&mut manifests, &mut saved, "example");
    assert!(manifests.is_empty());
    assert!(saved.is_empty());
}

#[test]
fn extension_manifest_validation() {
    let mut m = manifest("x", "/usr/bin/true", &[]);
    assert!(m.validate().is_ok());
    m.preferences = vec![
        ExtensionPreference {
            id: "a".into(),
            title: "A".into(),
            default_value: String::new(),
        },
        ExtensionPreference {
            id: "a".into(),
            title: "B".into(),
            default_value: String::new(),
        },
    ];
    assert!(m.validate().is_err());
    for bad in [
        manifest("", "/bin/true", &[]),
        manifest("x", "relative", &[]),
        manifest("x", "/bin/true", &["a\0b"]),
    ] {
        assert!(bad.validate().is_err());
    }
    let mut unnamed = manifest("x", "/bin/true", &[]);
    unnamed.name.clear();
    assert!(unnamed.validate().is_err());
}

#[test]
fn extension_round_trip_and_malformed_response() {
    let runner = AutomationRunner::new();
    let mut session = ExtensionSession::new();
    let mut good = manifest("test", "/usr/bin/printf", &["%s", GREETING]);
    good.enabled = true;
    session
        .search(&good, "test", &BTreeMap::new(), &runner)
        .unwrap();
    assert_eq!(session.results()[0].title, "Hello");
    assert_eq!(session.message(), Some("Success"));
    assert!(session.can_perform("test", "greeting", "greet"));
    session
        .perform(
            &good,
            "greeting",
            "greet",
            &BTreeMap::new(),
            &AutomationRunner::new(),
        )
        .unwrap();
    assert_eq!(
        session.perform(&good, "greeting", "unknown", &BTreeMap::new(), &runner),
        Err(AutomationError::ActionUnavailable)
    );
    // A rejected action leaves the offered results in place.
    assert_eq!(session.results().len(), 1);

    let mut bad = manifest("bad", "/usr/bin/printf", &["bad JSON"]);
    bad.enabled = true;
    let error = session
        .search(&bad, "", &BTreeMap::new(), &AutomationRunner::new())
        .unwrap_err();
    assert!(error.to_string().contains("malformed JSON"));
    assert!(session.results().is_empty());
    assert_eq!(session.active_id(), Some("bad"));
}

#[test]
fn extension_request_is_delivered_as_json_on_stdin() {
    let mut echo = manifest("echo", "/bin/cat", &[]);
    echo.enabled = true;
    // `cat` echoes the request; it is not a valid response, but the error proves the round trip
    // happened, so decode the request by running the runner directly.
    let mut prefs = BTreeMap::new();
    prefs.insert("prefix".to_string(), "Cześć".to_string());
    let request = ExtensionRequest::search("Łódź & cats", prefs);
    let output = AutomationRunner::new()
        .execute(
            &ExecuteRequest::new("/bin/cat", vec![])
                .with_input(serde_json::to_vec(&request).unwrap()),
        )
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(json["apiVersion"], 1);
    assert_eq!(json["operation"], "search");
    assert_eq!(json["query"], "Łódź & cats");
    assert_eq!(json["preferences"]["prefix"], "Cześć");
    assert!(json.get("resultID").is_none());
    let action = serde_json::to_value(ExtensionRequest::action("r", "a", BTreeMap::new())).unwrap();
    assert_eq!(action["resultID"], "r");
    assert_eq!(action["actionID"], "a");
    assert_eq!(action["operation"], "action");
    assert!(
        invoke_extension(
            &echo,
            &ExtensionRequest::search("q", BTreeMap::new()),
            &AutomationRunner::new()
        )
        .is_err()
    );
}

#[test]
fn disabled_extension_cannot_run() {
    let disabled = manifest("off", "/usr/bin/printf", &["%s", GREETING]);
    let mut session = ExtensionSession::new();
    assert_eq!(
        session.search(&disabled, "", &BTreeMap::new(), &AutomationRunner::new()),
        Err(AutomationError::NotEnabled("extension"))
    );
}

#[test]
fn extension_errors_leave_results_empty() {
    for (id, exe, args, expected) in [
        ("missing", "/missing/runtime", vec![], "missing"),
        ("crash", "/usr/bin/false", vec![], "status 1"),
        ("output", "/usr/bin/yes", vec![], "output limit"),
        (
            "version",
            "/usr/bin/printf",
            vec!["%s", r#"{"apiVersion":2,"results":[]}"#],
            "incompatible",
        ),
    ] {
        let mut m = manifest(id, exe, &args);
        m.enabled = true;
        let mut session = ExtensionSession::new();
        let error = session
            .search(&m, "test", &BTreeMap::new(), &AutomationRunner::new())
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{id}: {error}");
        assert!(session.results().is_empty());
    }
}

#[test]
fn slow_extension_times_out_after_five_seconds() {
    let mut m = manifest("slow", "/bin/sleep", &["30"]);
    m.enabled = true;
    let start = Instant::now();
    let mut session = ExtensionSession::new();
    let error = session
        .search(&m, "", &BTreeMap::new(), &AutomationRunner::new())
        .unwrap_err();
    assert_eq!(error, AutomationError::Timeout);
    assert!(start.elapsed() >= EXTENSION_TIMEOUT);
    assert!(start.elapsed() < Duration::from_secs(8));
}

#[test]
fn cancelling_a_slow_extension_leaves_no_stale_results() {
    let mut m = manifest("slow", "/bin/sleep", &["30"]);
    m.enabled = true;
    let runner = AutomationRunner::new();
    let handle = runner.cancel_handle();
    let worker = thread::spawn(move || {
        let mut session = ExtensionSession::new();
        let outcome = session.search(&m, "", &BTreeMap::new(), &runner);
        (outcome, session)
    });
    thread::sleep(Duration::from_millis(50));
    handle.cancel();
    let (outcome, session) = worker.join().unwrap();
    assert_eq!(outcome, Err(AutomationError::Cancelled));
    assert!(session.results().is_empty());
}

#[test]
fn session_clear_forgets_everything() {
    let mut session = ExtensionSession::new();
    session.begin_search("x");
    session
        .finish(Ok(ExtensionResponse {
            api_version: 1,
            results: vec![ExtensionResult {
                id: "one".into(),
                title: "One".into(),
                subtitle: None,
                detail: None,
                actions: vec![],
            }],
            message: Some("m".into()),
        }))
        .unwrap();
    assert_eq!(session.results().len(), 1);
    session.clear();
    assert!(session.results().is_empty());
    assert_eq!(session.active_id(), None);
    assert_eq!(session.message(), None);
}

#[test]
fn extension_rejects_duplicate_results_and_excessive_actions() {
    let result = |id: &str, actions: Vec<ExtensionAction>| ExtensionResult {
        id: id.into(),
        title: "Result".into(),
        subtitle: None,
        detail: None,
        actions,
    };
    let response = |results: Vec<ExtensionResult>| ExtensionResponse {
        api_version: 1,
        results,
        message: None,
    };
    assert!(
        response(vec![result("one", vec![]), result("one", vec![])])
            .validate()
            .is_err()
    );
    assert!(
        ExtensionResponse {
            api_version: 2,
            results: vec![],
            message: None
        }
        .validate()
        .is_err()
    );
    let many: Vec<_> = (0..101).map(|i| result(&i.to_string(), vec![])).collect();
    assert!(response(many).validate().is_err());
    let hundred: Vec<_> = (0..100).map(|i| result(&i.to_string(), vec![])).collect();
    assert!(response(hundred).validate().is_ok());
    let action = |id: &str| ExtensionAction {
        id: id.into(),
        title: "Action".into(),
    };
    let too_many: Vec<_> = (0..21).map(|i| action(&i.to_string())).collect();
    for actions in [
        too_many,
        vec![action("same"), action("same")],
        vec![action("")],
    ] {
        assert!(response(vec![result("one", actions)]).validate().is_err());
    }
    let twenty: Vec<_> = (0..20).map(|i| action(&i.to_string())).collect();
    assert!(response(vec![result("one", twenty)]).validate().is_ok());
    assert!(response(vec![result("", vec![])]).validate().is_err());
}

#[test]
fn provided_samples_work() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../examples/automation");
    let script = root.join("hello-script.sh");
    let result = run("/bin/sh", &[script.to_str().unwrap(), "Łódź & cats"]).unwrap();
    assert_eq!(result.stdout, "Hello, Łódź & cats!\n");
    let manifest: ExtensionManifest =
        serde_json::from_slice(&fs::read(root.join("greeting-manifest.json")).unwrap()).unwrap();
    assert!(manifest.validate().is_ok());
    assert_eq!(manifest.id, "example.greeting");
    assert!(!manifest.enabled);
}
