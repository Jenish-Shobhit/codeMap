//! `herdr-plugin.toml` checked against the rules herdr 0.9.0 applies when it
//! links a plugin (src/app/api/plugins/manifest.rs), without linking it.

use std::path::Path;

#[derive(serde::Deserialize)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    min_herdr_version: Option<String>,
    platforms: Option<Vec<String>>,
    #[serde(default)]
    build: Vec<Cmd>,
    #[serde(default)]
    actions: Vec<Action>,
    #[serde(default)]
    events: Vec<Event>,
    #[serde(default)]
    panes: Vec<Pane>,
}

#[derive(serde::Deserialize)]
struct Cmd {
    command: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Action {
    id: String,
    title: String,
    #[serde(default)]
    contexts: Vec<String>,
    command: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Event {
    on: String,
    command: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Pane {
    id: String,
    title: String,
    #[serde(default)]
    placement: Option<String>,
    width: Option<toml::Value>,
    height: Option<toml::Value>,
    command: Vec<String>,
}

fn plugin_id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
}

fn local_id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

fn version_tuple(v: &str) -> (u32, u32, u32) {
    let mut it = v.split('.').map(|p| p.parse::<u32>().unwrap());
    (it.next().unwrap(), it.next().unwrap(), it.next().unwrap())
}

/// herdr resolves a relative argv[0] containing a slash against the plugin
/// root (plugin_command::program_for_cwd); a bare name is looked up on PATH.
fn resolves_to_our_binary(cmd: &[String]) -> bool {
    let program = &cmd[0];
    program.contains('/')
        && Path::new(program).is_relative()
        && program == "target/release/codemorph"
}

#[test]
fn manifest_passes_herdr_validation() {
    let text =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/herdr-plugin.toml")).unwrap();
    let m: Manifest = toml::from_str(&text).unwrap();
    assert_eq!(m.id, "dev.codemorph");
    assert!(plugin_id_ok(&m.id));
    assert_eq!(m.name, "codeMorph");
    assert!(!m.version.trim().is_empty());
    assert_eq!(
        m.version,
        env!("CARGO_PKG_VERSION"),
        "manifest and crate versions agree"
    );
    let min = m
        .min_herdr_version
        .as_deref()
        .expect("min_herdr_version is required");
    assert_eq!(min, "0.9.0");
    assert!(
        version_tuple(min) <= (0, 9, 0),
        "must not require a newer herdr than 0.9.0"
    );
    let platforms = m
        .platforms
        .expect("declare platforms to avoid a link warning");
    assert!(!platforms.is_empty());
    for p in &platforms {
        assert!(["linux", "macos", "windows"].contains(&p.as_str()));
    }

    for b in &m.build {
        assert!(!b.command.is_empty() && b.command.iter().all(|a| !a.is_empty()));
    }
    let mut seen = std::collections::HashSet::new();
    for a in &m.actions {
        assert!(local_id_ok(&a.id), "action id {}", a.id);
        assert!(seen.insert(a.id.clone()), "duplicate action id {}", a.id);
        assert!(!a.title.trim().is_empty());
        for c in &a.contexts {
            assert!(
                ["global", "workspace", "tab", "pane", "selection"].contains(&c.as_str()),
                "context {c}"
            );
        }
        assert!(resolves_to_our_binary(&a.command), "{:?}", a.command);
        assert_eq!(a.command[1], "open");
    }
    for want in ["open", "flow", "changes", "history"] {
        assert!(
            m.actions.iter().any(|a| a.id == want),
            "missing action {want}"
        );
    }

    // The checkpoint hook listens to a known event name.
    assert_eq!(m.events.len(), 1);
    assert_eq!(m.events[0].on, "pane.agent_status_changed");
    assert_eq!(
        m.events[0].command,
        vec!["target/release/codemorph", "hook"]
    );

    // The popup entrypoint at 94% x 92%; width/height only valid on popups.
    assert_eq!(m.panes.len(), 1);
    let p = &m.panes[0];
    assert!(local_id_ok(&p.id));
    assert_eq!(p.id, "ui");
    assert_eq!(p.title, "codeMorph");
    assert_eq!(p.placement.as_deref(), Some("popup"));
    assert_eq!(p.width.as_ref().and_then(|v| v.as_str()), Some("94%"));
    assert_eq!(p.height.as_ref().and_then(|v| v.as_str()), Some("92%"));
    for size in [&p.width, &p.height] {
        let s = size.as_ref().unwrap().as_str().unwrap();
        let n: u32 = s.trim_end_matches('%').parse().unwrap();
        assert!(
            (1..=100).contains(&n),
            "PopupSize pattern ^(100|[1-9][0-9]?)%$"
        );
    }
    assert!(resolves_to_our_binary(&p.command));
    assert_eq!(p.command[1], "ui");
}

#[test]
fn binary_answers_version_and_help() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_codemorph"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("codemorph "));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_codemorph"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("codemorph open"));
    // `open` outside herdr fails cleanly instead of hanging.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_codemorph"))
        .arg("open")
        .env_remove("HERDR_SOCKET_PATH")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("HERDR_SOCKET_PATH"));
}
