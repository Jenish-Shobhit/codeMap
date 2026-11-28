use std::path::{Path, PathBuf};

use serde_json::Value;

const EXCERPT_LINES: usize = 8;
const DESC_CHARS: usize = 320;
const BACKUP_WORDS: [&str; 5] = [".bak", ".before-", ".backup", ".orig", ".old"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Project,
    Global,
    Tools,
    Agents,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Project, Scope::Global, Scope::Tools, Scope::Agents];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
            Scope::Tools => "tools",
            Scope::Agents => "other agents",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub repo: PathBuf,
    pub home: PathBuf,
    pub config: PathBuf,
}

impl Roots {
    pub fn from_env(repo: &Path) -> Roots {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Roots {
            repo: repo.to_path_buf(),
            home,
            config,
        }
    }

    pub fn first_scope(&self) -> Scope {
        if self.repo.join(".claude").is_dir() || self.repo.join("CLAUDE.md").is_file() {
            Scope::Project
        } else {
            Scope::Global
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    pub name: String,
    pub hint: String,
    pub kind: String,
    pub path: PathBuf,
    pub dir: bool,
    pub desc: Option<String>,
    pub facts: Vec<(String, String)>,
    pub excerpt: Vec<(usize, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Group {
    pub title: String,
    pub items: Vec<Item>,
}

fn group(title: &str, items: Vec<Item>) -> Group {
    Group {
        title: title.to_string(),
        items,
    }
}

pub fn scan(scope: Scope, roots: &Roots) -> Vec<Group> {
    match scope {
        Scope::Project => project(roots),
        Scope::Global => global(roots),
        Scope::Tools => tools(roots),
        Scope::Agents => other_agents(roots),
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn json(path: &Path) -> Option<Value> {
    serde_json::from_str(&read(path)?).ok()
}

fn toml_value(path: &Path) -> Option<toml::Value> {
    toml::from_str(&read(path)?).ok()
}

fn size_fact(text: &str) -> (String, String) {
    let lines = text.lines().count();
    let size = match lines {
        0 => "empty".to_string(),
        1 => "1 line".to_string(),
        n => format!("{n} lines"),
    };
    ("Size".to_string(), size)
}

fn fact(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

pub fn is_backup(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with('~') || BACKUP_WORDS.iter().any(|w| lower.contains(w))
}

fn backups_of(path: &Path) -> usize {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return 0;
    };
    let name = name.to_string_lossy();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|other| other.len() > name.len() && other.starts_with(&*name) && is_backup(other))
        .count()
}

fn with_backups(mut item: Item) -> Item {
    let hidden = backups_of(&item.path);
    if hidden > 0 {
        item.facts.push(fact("Backups", format!("{hidden} hidden")));
    }
    item
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    let quoted = v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
    if quoted {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

pub fn frontmatter(text: &str) -> (Vec<(String, String)>, usize) {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return (Vec::new(), 0);
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut parent = String::new();
    let mut folding = false;
    for (i, line) in lines.enumerate() {
        if line.trim_end() == "---" {
            return (pairs, i + 2);
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented && folding {
            if let Some(last) = pairs.last_mut() {
                if !last.1.is_empty() {
                    last.1.push(' ');
                }
                last.1.push_str(line.trim());
            }
            continue;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        folding = matches!(value, ">" | "|" | ">-" | "|-");
        let name = if indented && !parent.is_empty() {
            format!("{parent}.{key}")
        } else {
            key.to_string()
        };
        if !indented {
            parent = if value.is_empty() {
                key.to_string()
            } else {
                String::new()
            };
        }
        if folding {
            pairs.push((name, String::new()));
        } else if !value.is_empty() {
            pairs.push((name, unquote(value)));
        }
    }
    (Vec::new(), 0)
}

fn field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

fn first_paragraph(text: &str, skip: usize) -> Option<String> {
    text.lines()
        .skip(skip)
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("<!--"))
        .map(|l| clip(l, DESC_CHARS))
}

fn headings(text: &str, skip: usize) -> Vec<String> {
    text.lines()
        .skip(skip)
        .filter_map(|l| l.strip_prefix("## "))
        .map(|h| h.trim().to_string())
        .collect()
}

fn excerpt(text: &str, skip: usize) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .skip(skip)
        .skip_while(|(_, l)| l.trim().is_empty())
        .take(EXCERPT_LINES)
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    out.retain(|p| !is_backup(&file_name(p)) && !file_name(p).starts_with(".DS_"));
    out.sort();
    out
}

fn markdown_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in sorted_entries(dir) {
        if path.is_dir() {
            out.extend(markdown_under(&path));
        } else if path.extension().is_some_and(|e| e == "md" || e == "mdc") {
            out.push(path);
        }
    }
    out
}

fn instructions(path: &Path, where_: &str) -> Option<Item> {
    let text = read(path)?;
    let (_, skip) = frontmatter(&text);
    let sections = headings(&text, skip);
    let mut facts = Vec::new();
    if !sections.is_empty() {
        let shown: Vec<&str> = sections.iter().take(4).map(String::as_str).collect();
        let more = sections.len().saturating_sub(shown.len());
        let tail = if more > 0 {
            format!(" · {more} more")
        } else {
            String::new()
        };
        facts.push(fact("Sections", format!("{}{tail}", shown.join(" · "))));
    }
    facts.push(size_fact(&text));
    let empty = text.trim().is_empty();
    Some(with_backups(Item {
        name: file_name(path),
        hint: if empty { "empty" } else { "instructions" }.to_string(),
        kind: format!("instructions · {where_}"),
        path: path.to_path_buf(),
        desc: if empty {
            Some("Empty file. Nothing is loaded from it.".to_string())
        } else {
            first_paragraph(&text, skip)
        },
        facts,
        excerpt: excerpt(&text, skip),
        ..Default::default()
    }))
}

fn described(path: &Path, kind: &str, name: String, extra: &[(&str, &str)]) -> Option<Item> {
    let text = read(path)?;
    let (pairs, skip) = frontmatter(&text);
    let desc = field(&pairs, "description")
        .map(|d| clip(d, DESC_CHARS))
        .or_else(|| first_paragraph(&text, skip));
    let mut facts = Vec::new();
    for (key, label) in extra {
        if let Some(v) = field(&pairs, key) {
            facts.push(fact(label, clip(v, 120)));
        }
    }
    facts.push(size_fact(&text));
    Some(Item {
        name: field(&pairs, "name").map(str::to_string).unwrap_or(name),
        hint: desc.clone().unwrap_or_default(),
        kind: kind.to_string(),
        path: path.to_path_buf(),
        desc,
        facts,
        excerpt: excerpt(&text, skip),
        ..Default::default()
    })
}

fn skills(dir: &Path, where_: &str) -> Vec<Item> {
    sorted_entries(&dir.join("skills"))
        .into_iter()
        .filter_map(|d| {
            let file = d.join("SKILL.md");
            described(&file, &format!("skill · {where_}"), file_name(&d), &[])
        })
        .collect()
}

fn agents(dir: &Path, where_: &str) -> Vec<Item> {
    markdown_under(&dir.join("agents"))
        .into_iter()
        .filter_map(|p| {
            described(
                &p,
                &format!("agent · {where_}"),
                stem(&p),
                &[("model", "Model"), ("tools", "Tools")],
            )
        })
        .collect()
}

fn commands(dir: &Path, where_: &str) -> Vec<Item> {
    let base = dir.join("commands");
    markdown_under(&base)
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(&base).unwrap_or(&p).with_extension("");
            let name = format!("/{}", rel.to_string_lossy().replace('/', ":"));
            let mut item = described(
                &p,
                &format!("command · {where_}"),
                name.clone(),
                &[("argument-hint", "Arguments"), ("allowed-tools", "Tools")],
            )?;
            item.name = name;
            Some(item)
        })
        .collect()
}

fn count_of(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Object(o)) => o.len(),
        _ => 0,
    }
}

fn settings(path: &Path, where_: &str) -> Option<Item> {
    let text = read(path)?;
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Some(Item {
            name: file_name(path),
            hint: "not valid JSON".to_string(),
            kind: format!("settings · {where_}"),
            path: path.to_path_buf(),
            desc: Some("This file does not parse as JSON, so nothing is read from it.".to_string()),
            facts: vec![size_fact(&text)],
            ..Default::default()
        });
    };
    let mut facts = Vec::new();
    let mut hint = Vec::new();
    if let Some(model) = value.get("model").and_then(Value::as_str) {
        facts.push(fact("Model", model));
        hint.push(model.to_string());
    }
    if let Some(p) = value.get("permissions") {
        let mut parts = Vec::new();
        if let Some(mode) = p.get("defaultMode").and_then(Value::as_str) {
            parts.push(format!("default mode {mode}"));
            hint.push(format!("{mode} mode"));
        }
        for list in ["allow", "ask", "deny"] {
            let n = count_of(p.get(list));
            if n > 0 {
                parts.push(format!("{n} {list}"));
            }
        }
        if !parts.is_empty() {
            facts.push(fact("Permissions", parts.join(" · ")));
        }
    }
    let hooks = hook_items(&value, path, where_).len();
    if hooks > 0 {
        facts.push(fact("Hooks", hooks.to_string()));
    }
    if let Some(Value::Object(plugins)) = value.get("enabledPlugins") {
        let on = plugins
            .values()
            .filter(|v| v.as_bool() == Some(true))
            .count();
        facts.push(fact("Plugins", format!("{on} enabled")));
    }
    if value.get("statusLine").is_some() {
        facts.push(fact("Status line", "custom"));
    }
    if let Some(Value::Object(env)) = value.get("env") {
        let names: Vec<&str> = env.keys().map(String::as_str).collect();
        facts.push(fact("Environment", clip(&names.join(", "), 120)));
    }
    facts.push(fact(
        "Size",
        format!(
            "{} lines, {} keys",
            text.lines().count(),
            count_of(Some(&value))
        ),
    ));
    Some(with_backups(Item {
        name: file_name(path),
        hint: hint.join(" · "),
        kind: format!("settings · {where_}"),
        path: path.to_path_buf(),
        facts,
        ..Default::default()
    }))
}

fn hook_items(settings: &Value, path: &Path, where_: &str) -> Vec<Item> {
    let Some(Value::Object(events)) = settings.get("hooks") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (event, entries) in events {
        for entry in entries.as_array().into_iter().flatten() {
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .unwrap_or("every call");
            for hook in entry
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let kind = hook
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("command");
                let runs = hook
                    .get("command")
                    .or_else(|| hook.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push(Item {
                    name: event.clone(),
                    hint: matcher.to_string(),
                    kind: format!("hook · {where_}"),
                    path: path.to_path_buf(),
                    desc: Some(clip(runs, DESC_CHARS)),
                    facts: vec![
                        fact("When", event.as_str()),
                        fact("Matches", matcher),
                        fact("Type", kind),
                    ],
                    ..Default::default()
                });
            }
        }
    }
    out
}

fn hooks(paths: &[PathBuf], where_: &str) -> Vec<Item> {
    paths
        .iter()
        .filter_map(|p| Some((json(p)?, p)))
        .flat_map(|(value, p)| hook_items(&value, p, where_))
        .collect()
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host.rsplit('@').next().unwrap_or(host).to_string()
}

fn mcp_items(servers: Option<&Value>, path: &Path, where_: &str) -> Vec<Item> {
    let Some(Value::Object(servers)) = servers else {
        return Vec::new();
    };
    servers
        .iter()
        .map(|(name, server)| {
            let url = server.get("url").and_then(Value::as_str);
            let command = server.get("command").and_then(Value::as_str);
            let transport = server
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or(if url.is_some() { "http" } else { "stdio" });
            let mut facts = vec![fact("Transport", transport)];
            if let Some(command) = command {
                facts.push(fact("Runs", file_name(Path::new(command))));
                let args = count_of(server.get("args"));
                if args > 0 {
                    facts.push(fact("Arguments", args.to_string()));
                }
            }
            if let Some(url) = url {
                facts.push(fact("Host", host_of(url)));
            }
            if let Some(Value::Object(env)) = server.get("env") {
                let names: Vec<&str> = env.keys().map(String::as_str).collect();
                facts.push(fact("Environment", clip(&names.join(", "), 120)));
            }
            Item {
                name: name.clone(),
                hint: transport.to_string(),
                kind: format!("MCP server · {where_}"),
                path: path.to_path_buf(),
                facts,
                ..Default::default()
            }
        })
        .collect()
}

fn plugins(settings_path: &Path) -> Vec<Item> {
    let Some(value) = json(settings_path) else {
        return Vec::new();
    };
    let Some(Value::Object(plugins)) = value.get("enabledPlugins") else {
        return Vec::new();
    };
    plugins
        .iter()
        .map(|(id, on)| {
            let (name, market) = id.split_once('@').unwrap_or((id, ""));
            let enabled = on.as_bool() == Some(true);
            let mut facts = vec![fact("Status", if enabled { "enabled" } else { "disabled" })];
            if !market.is_empty() {
                facts.push(fact("Marketplace", market));
            }
            Item {
                name: name.to_string(),
                hint: if enabled { market } else { "disabled" }.to_string(),
                kind: "plugin · global".to_string(),
                path: settings_path.to_path_buf(),
                facts,
                ..Default::default()
            }
        })
        .collect()
}

pub fn project_key(repo: &Path) -> String {
    repo.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn memory(roots: &Roots) -> Vec<Item> {
    let dir = roots
        .home
        .join(".claude/projects")
        .join(project_key(&roots.repo))
        .join("memory");
    markdown_under(&dir)
        .into_iter()
        .filter(|p| file_name(p) != "MEMORY.md")
        .filter_map(|p| {
            let text = read(&p)?;
            let (pairs, skip) = frontmatter(&text);
            let kind = field(&pairs, "metadata.type")
                .or_else(|| field(&pairs, "type"))
                .unwrap_or("memory");
            let mut facts = vec![fact("Type", kind)];
            if let Some(modified) = field(&pairs, "metadata.modified") {
                facts.push(fact(
                    "Modified",
                    modified.split('T').next().unwrap_or(modified),
                ));
            }
            facts.push(size_fact(&text));
            Some(Item {
                name: field(&pairs, "name")
                    .map(str::to_string)
                    .unwrap_or(stem(&p)),
                hint: kind.to_string(),
                kind: "memory · project".to_string(),
                path: p.clone(),
                desc: field(&pairs, "description")
                    .map(|d| clip(d, DESC_CHARS))
                    .or_else(|| first_paragraph(&text, skip)),
                facts,
                excerpt: excerpt(&text, skip),
                ..Default::default()
            })
        })
        .collect()
}

fn existing(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().filter(|p| p.is_file()).collect()
}

fn project(roots: &Roots) -> Vec<Group> {
    let repo = &roots.repo;
    let dir = repo.join(".claude");
    let settings_files = existing(vec![
        dir.join("settings.json"),
        dir.join("settings.local.json"),
    ]);
    let mut servers = mcp_items(
        json(&repo.join(".mcp.json"))
            .as_ref()
            .and_then(|v| v.get("mcpServers")),
        &repo.join(".mcp.json"),
        "project",
    );
    let user_file = roots.home.join(".claude.json");
    if let Some(user) = json(&user_file) {
        let local = user
            .get("projects")
            .and_then(|p| p.get(repo.to_string_lossy().as_ref()))
            .and_then(|p| p.get("mcpServers"));
        servers.extend(mcp_items(local, &user_file, "this project only"));
    }
    vec![
        group(
            "Instructions",
            [
                repo.join("CLAUDE.md"),
                repo.join("CLAUDE.local.md"),
                dir.join("CLAUDE.md"),
            ]
            .iter()
            .filter_map(|p| instructions(p, "project"))
            .collect(),
        ),
        group(
            "Settings",
            settings_files
                .iter()
                .filter_map(|p| settings(p, "project"))
                .collect(),
        ),
        group("Skills", skills(&dir, "project")),
        group("Agents", agents(&dir, "project")),
        group("Commands", commands(&dir, "project")),
        group("Hooks", hooks(&settings_files, "project")),
        group("MCP servers", servers),
        group("Memory", memory(roots)),
    ]
}

fn global(roots: &Roots) -> Vec<Group> {
    let dir = roots.home.join(".claude");
    let settings_file = dir.join("settings.json");
    let user_file = roots.home.join(".claude.json");
    let mut settings_items: Vec<Item> = settings(&settings_file, "global").into_iter().collect();
    let keybindings = dir.join("keybindings.json");
    if let Some(text) = read(&keybindings) {
        settings_items.push(Item {
            name: "keybindings.json".to_string(),
            hint: "keyboard shortcuts".to_string(),
            kind: "settings · global".to_string(),
            path: keybindings,
            facts: vec![size_fact(&text)],
            ..Default::default()
        });
    }
    vec![
        group(
            "Instructions",
            instructions(&dir.join("CLAUDE.md"), "global")
                .into_iter()
                .collect(),
        ),
        group("Settings", settings_items),
        group("Plugins", plugins(&settings_file)),
        group("Skills", skills(&dir, "global")),
        group("Agents", agents(&dir, "global")),
        group("Commands", commands(&dir, "global")),
        group("Hooks", hooks(&existing(vec![settings_file]), "global")),
        group(
            "MCP servers",
            mcp_items(
                json(&user_file).as_ref().and_then(|v| v.get("mcpServers")),
                &user_file,
                "global",
            ),
        ),
    ]
}

fn config_item(path: &Path, hint: String, facts: Vec<(String, String)>) -> Option<Item> {
    let text = read(path)?;
    let mut facts = facts;
    facts.push(size_fact(&text));
    Some(with_backups(Item {
        name: file_name(path),
        hint,
        kind: "tool config".to_string(),
        path: path.to_path_buf(),
        facts,
        ..Default::default()
    }))
}

fn plain(path: &Path) -> Option<Item> {
    config_item(path, String::new(), Vec::new())
}

fn setting_lines(path: &Path, separator: char, keys: &[(&str, &str)]) -> Vec<(String, String)> {
    let Some(text) = read(path) else {
        return Vec::new();
    };
    let mut facts = Vec::new();
    for (key, label) in keys {
        let found = text.lines().find_map(|line| {
            let line = line.trim();
            let (k, v) = line.split_once(separator)?;
            (k.trim() == *key).then(|| unquote(v))
        });
        if let Some(value) = found.filter(|v| !v.is_empty()) {
            facts.push(fact(label, clip(&value, 80)));
        }
    }
    facts
}

fn toml_str<'a>(value: &'a toml::Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |v, key| v.get(key))
        .and_then(toml::Value::as_str)
}

fn toml_len(value: &toml::Value, path: &[&str]) -> usize {
    match path.iter().try_fold(value, |v, key| v.get(key)) {
        Some(toml::Value::Array(a)) => a.len(),
        Some(toml::Value::Table(t)) => t.len(),
        _ => 0,
    }
}

fn herdr(config: &Path) -> Vec<Item> {
    let dir = config.join("herdr");
    let file = dir.join("config.toml");
    let mut out = Vec::new();
    let mut facts = Vec::new();
    let mut hint = String::new();
    if let Some(v) = toml_value(&file) {
        if let Some(theme) = toml_str(&v, &["theme", "name"]) {
            let accent = toml_str(&v, &["theme", "custom", "accent"]);
            let custom = accent
                .map(|a| format!(", custom accent {a}"))
                .unwrap_or_default();
            facts.push(fact("Theme", format!("{theme}{custom}")));
            hint = format!("theme {theme}");
        }
        if let Some(prefix) = toml_str(&v, &["keys", "prefix"]) {
            facts.push(fact("Prefix key", prefix));
        }
        let commands = toml_len(&v, &["keys", "command"]);
        if commands > 0 {
            facts.push(fact("Key commands", commands.to_string()));
        }
    }
    out.extend(config_item(&file, hint, facts));
    let registry = dir.join("plugins.json");
    if let Some(Value::Array(list)) = json(&registry) {
        let names: Vec<&str> = list
            .iter()
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .collect();
        out.extend(config_item(
            &registry,
            format!("{} plugins", names.len()),
            vec![fact("Plugins", clip(&names.join(", "), 160))],
        ));
    }
    out
}

fn yazi(config: &Path) -> Vec<Item> {
    let dir = config.join("yazi");
    let mut out = Vec::new();
    let main = dir.join("yazi.toml");
    let mut facts = Vec::new();
    if let Some(v) = toml_value(&main) {
        if let Some(toml::Value::Table(openers)) = v.get("opener") {
            let names: Vec<&str> = openers.keys().map(String::as_str).collect();
            facts.push(fact("Openers", names.join(", ")));
        }
        let rules = toml_len(&v, &["open", "prepend_rules"]);
        if rules > 0 {
            facts.push(fact("Open rules", rules.to_string()));
        }
    }
    let hint = facts
        .first()
        .map(|(_, v)| format!("opens with {v}"))
        .unwrap_or_default();
    out.extend(config_item(&main, hint, facts));
    let keymap = dir.join("keymap.toml");
    let keys = toml_value(&keymap)
        .map(|v| toml_len(&v, &["mgr", "prepend_keymap"]))
        .unwrap_or(0);
    out.extend(config_item(
        &keymap,
        format!("{keys} keys"),
        vec![fact("Added keys", keys.to_string())],
    ));
    for name in ["theme.toml", "init.lua", "package.toml"] {
        out.extend(plain(&dir.join(name)));
    }
    let plugin_dir = dir.join("plugins");
    let plugin_names: Vec<String> = sorted_entries(&plugin_dir)
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| stem(p))
        .collect();
    if !plugin_names.is_empty() {
        out.push(Item {
            name: "plugins".to_string(),
            hint: format!("{} installed", plugin_names.len()),
            kind: "tool config".to_string(),
            path: plugin_dir,
            dir: true,
            facts: vec![fact("Plugins", plugin_names.join(", "))],
            ..Default::default()
        });
    }
    out
}

fn tools(roots: &Roots) -> Vec<Group> {
    let (home, config) = (&roots.home, &roots.config);
    let neovim: Vec<Item> = ["init.lua", "init.vim"]
        .iter()
        .filter_map(|name| {
            let path = config.join("nvim").join(name);
            let first = read(&path)?
                .lines()
                .map(|l| l.trim_start_matches(['-', '"', ' ']).trim().to_string())
                .find(|l| l.chars().any(char::is_alphanumeric));
            let mut item = plain(&path)?;
            item.desc = first.map(|l| clip(&l, DESC_CHARS));
            Some(item)
        })
        .collect();
    let ghostty_file = config.join("ghostty/config");
    let ghostty_facts = setting_lines(
        &ghostty_file,
        '=',
        &[
            ("theme", "Theme"),
            ("font-family", "Font"),
            ("font-size", "Font size"),
        ],
    );
    let ghostty_hint = ghostty_facts
        .first()
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let kitty_file = config.join("kitty/kitty.conf");
    let kitty_facts = setting_lines(
        &kitty_file,
        ' ',
        &[("font_family", "Font"), ("font_size", "Font size")],
    );
    let shell: Vec<Item> = [
        home.join(".zshrc"),
        home.join(".zprofile"),
        home.join(".bashrc"),
        config.join("fish/config.fish"),
    ]
    .iter()
    .filter_map(|p| plain(p))
    .collect();
    let groups = vec![
        group("herdr", herdr(config)),
        group("yazi", yazi(config)),
        group("neovim", neovim),
        group(
            "ghostty",
            config_item(&ghostty_file, ghostty_hint, ghostty_facts)
                .into_iter()
                .collect(),
        ),
        group(
            "kitty",
            config_item(&kitty_file, String::new(), kitty_facts)
                .into_iter()
                .collect(),
        ),
        group(
            "tmux",
            [home.join(".tmux.conf"), config.join("tmux/tmux.conf")]
                .iter()
                .filter_map(|p| plain(p))
                .collect(),
        ),
        group("shell", shell),
        group(
            "git",
            [home.join(".gitconfig"), config.join("git/config")]
                .iter()
                .filter_map(|p| plain(p))
                .collect(),
        ),
    ];
    groups.into_iter().filter(|g| !g.items.is_empty()).collect()
}

fn agent_file(path: &Path, kind: &str) -> Option<Item> {
    if path.extension().is_some_and(|e| e == "md" || e == "mdc") || file_name(path).starts_with('.')
    {
        let mut item = instructions(path, kind)?;
        item.kind = kind.to_string();
        return Some(item);
    }
    let mut item = plain(path)?;
    item.kind = kind.to_string();
    Some(item)
}

fn other_agents(roots: &Roots) -> Vec<Group> {
    let (repo, home) = (&roots.repo, &roots.home);
    let mut here: Vec<PathBuf> = vec![
        repo.join("AGENTS.md"),
        repo.join("GEMINI.md"),
        repo.join(".cursorrules"),
        repo.join(".windsurfrules"),
        repo.join(".github/copilot-instructions.md"),
        repo.join(".codex/config.toml"),
    ];
    here.extend(markdown_under(&repo.join(".cursor/rules")));
    let mut groups = vec![group(
        "In this repository",
        here.iter()
            .filter_map(|p| {
                let mut item = agent_file(p, "instructions · this repository")?;
                item.name = p
                    .strip_prefix(repo)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .into_owned();
                Some(item)
            })
            .collect(),
    )];
    let codex = home.join(".codex");
    if codex.is_dir() {
        let mut items: Vec<Item> = agent_file(&codex.join("AGENTS.md"), "instructions · Codex")
            .into_iter()
            .collect();
        let config = codex.join("config.toml");
        let mut facts = Vec::new();
        if let Some(v) = toml_value(&config) {
            for (key, label) in [
                ("model", "Model"),
                ("approval_policy", "Approvals"),
                ("sandbox_mode", "Sandbox"),
            ] {
                if let Some(value) = toml_str(&v, &[key]) {
                    facts.push(fact(label, value));
                }
            }
            let servers = toml_len(&v, &["mcp_servers"]);
            if servers > 0 {
                facts.push(fact("MCP servers", servers.to_string()));
            }
        }
        let hint = facts.first().map(|(_, v)| v.clone()).unwrap_or_default();
        if let Some(mut item) = config_item(&config, hint, facts) {
            item.kind = "settings · Codex".to_string();
            items.push(item);
        }
        groups.push(group("Codex", items));
    }
    let cursor = home.join(".cursor");
    if cursor.is_dir() {
        let mut items = Vec::new();
        let mcp = cursor.join("mcp.json");
        if let Some(v) = json(&mcp) {
            let servers = count_of(v.get("mcpServers"));
            if let Some(mut item) = config_item(
                &mcp,
                format!("{servers} servers"),
                vec![fact("MCP servers", servers.to_string())],
            ) {
                item.kind = "settings · Cursor".to_string();
                items.push(item);
            }
        }
        items.extend(
            markdown_under(&cursor.join("rules"))
                .iter()
                .filter_map(|p| agent_file(p, "instructions · Cursor")),
        );
        groups.push(group("Cursor", items));
    }
    let gemini = home.join(".gemini");
    if gemini.is_dir() {
        let mut items: Vec<Item> = agent_file(&gemini.join("GEMINI.md"), "instructions · Gemini")
            .into_iter()
            .collect();
        if let Some(mut item) = plain(&gemini.join("settings.json")) {
            item.kind = "settings · Gemini".to_string();
            items.push(item);
        }
        groups.push(group("Gemini", items));
    }
    groups
}

#[derive(Debug, Default)]
pub struct CustomState {
    pub roots: Option<Roots>,
    pub loaded: bool,
    pub scope: Scope,
    pub scanned: Vec<(Scope, Vec<Group>)>,
    pub sel: [usize; 4],
    pub scroll: usize,
}

impl CustomState {
    fn slot(scope: Scope) -> usize {
        Scope::ALL.iter().position(|s| *s == scope).unwrap_or(0)
    }

    pub fn groups(&self) -> &[Group] {
        self.scanned
            .iter()
            .find(|(s, _)| *s == self.scope)
            .map(|(_, g)| g.as_slice())
            .unwrap_or(&[])
    }

    pub fn items(&self) -> Vec<&Item> {
        self.groups().iter().flat_map(|g| &g.items).collect()
    }

    pub fn selected_index(&self) -> usize {
        self.sel[Self::slot(self.scope)]
    }

    pub fn selected(&self) -> Option<&Item> {
        self.items().get(self.selected_index()).copied()
    }

    pub fn move_by(&mut self, delta: i32) {
        let count = self.items().len();
        if count == 0 {
            return;
        }
        let slot = Self::slot(self.scope);
        self.sel[slot] = (self.sel[slot] as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
    }

    pub fn set_scope(&mut self, scope: Scope) {
        self.scope = scope;
        self.scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let base =
                std::env::temp_dir().join(format!("codemap-custom-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            Fixture(base.canonicalize().unwrap())
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn roots(&self) -> Roots {
            Roots {
                repo: self.0.join("repo"),
                home: self.0.join("home"),
                config: self.0.join("home/.config"),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn named<'a>(groups: &'a [Group], title: &str) -> &'a Group {
        groups
            .iter()
            .find(|g| g.title == title)
            .unwrap_or_else(|| panic!("no group {title}"))
    }

    fn fact_of<'a>(item: &'a Item, key: &str) -> Option<&'a str> {
        item.facts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn frontmatter_reads_plain_folded_and_nested_values() {
        let text = "---\nname: review\ndescription: >\n  Reads a diff\n  and comments.\nmetadata:\n  type: project\ntools: \"Read, Grep\"\n---\n\n# Body\n";
        let (pairs, skip) = frontmatter(text);
        assert_eq!(skip, 9);
        assert_eq!(field(&pairs, "name"), Some("review"));
        assert_eq!(
            field(&pairs, "description"),
            Some("Reads a diff and comments.")
        );
        assert_eq!(field(&pairs, "metadata.type"), Some("project"));
        assert_eq!(field(&pairs, "tools"), Some("Read, Grep"));
        assert_eq!(frontmatter("# No front matter\n"), (Vec::new(), 0));
        assert_eq!(frontmatter("---\nname: unterminated\n"), (Vec::new(), 0));
    }

    #[test]
    fn project_scope_lists_every_kind_and_keeps_empty_ones() {
        let f = Fixture::new("project");
        f.write(
            "repo/CLAUDE.md",
            "# demo\n\nA demo project.\n\n## Rules\n\nBe kind.\n\n## Releases\n",
        );
        f.write(
            "repo/.claude/skills/review/SKILL.md",
            "---\nname: review\ndescription: Review a diff for bugs.\n---\n\n# Review\n\nSteps.\n",
        );
        f.write(
            "repo/.claude/agents/planner.md",
            "---\nname: planner\ndescription: Plans the work.\nmodel: opus\ntools: Read, Grep\n---\nBody\n",
        );
        f.write("repo/.claude/commands/git/ship.md", "Ship the branch.\n");
        f.write(
            "repo/.claude/settings.json",
            r#"{"model":"sonnet","permissions":{"defaultMode":"plan","allow":["Bash(ls)","Read"],"deny":["Bash(rm *)"]},
               "env":{"API_TOKEN":"s3cret"},
               "hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"./check.sh"}]}]}}"#,
        );
        f.write(
            "repo/.mcp.json",
            r#"{"mcpServers":{"docs":{"type":"http","url":"https://user:pw@mcp.example.com/v1?key=abc"},
                              "local":{"command":"/usr/local/bin/serve","args":["--port","1"],"env":{"TOKEN":"x"}}}}"#,
        );
        let roots = f.roots();
        let key = project_key(&roots.repo);
        f.write(
            &format!("home/.claude/projects/{key}/memory/gate.md"),
            "---\nname: merge-gate\ndescription: main is PR-only.\nmetadata:\n  type: project\n  modified: 2026-11-27T18:17:18Z\n---\n\nFact.\n",
        );
        f.write(
            &format!("home/.claude/projects/{key}/memory/MEMORY.md"),
            "- index\n",
        );

        let groups = scan(Scope::Project, &roots);
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Instructions",
                "Settings",
                "Skills",
                "Agents",
                "Commands",
                "Hooks",
                "MCP servers",
                "Memory"
            ]
        );

        let claude = &named(&groups, "Instructions").items[0];
        assert_eq!(claude.name, "CLAUDE.md");
        assert_eq!(claude.desc.as_deref(), Some("A demo project."));
        assert_eq!(fact_of(claude, "Sections"), Some("Rules · Releases"));
        assert_eq!(fact_of(claude, "Size"), Some("9 lines"));

        let skill = &named(&groups, "Skills").items[0];
        assert_eq!(
            (skill.name.as_str(), skill.kind.as_str()),
            ("review", "skill · project")
        );
        assert_eq!(skill.desc.as_deref(), Some("Review a diff for bugs."));
        assert_eq!(skill.excerpt[0], (6, "# Review".to_string()));

        let agent = &named(&groups, "Agents").items[0];
        assert_eq!(fact_of(agent, "Model"), Some("opus"));
        assert_eq!(fact_of(agent, "Tools"), Some("Read, Grep"));

        let command = &named(&groups, "Commands").items[0];
        assert_eq!(command.name, "/git:ship");
        assert_eq!(command.desc.as_deref(), Some("Ship the branch."));

        let settings = &named(&groups, "Settings").items[0];
        assert_eq!(fact_of(settings, "Model"), Some("sonnet"));
        assert_eq!(
            fact_of(settings, "Permissions"),
            Some("default mode plan · 2 allow · 1 deny")
        );
        assert_eq!(fact_of(settings, "Environment"), Some("API_TOKEN"));
        assert_eq!(settings.hint, "sonnet · plan mode");

        let hook = &named(&groups, "Hooks").items[0];
        assert_eq!(
            (hook.name.as_str(), hook.hint.as_str()),
            ("PreToolUse", "Bash")
        );
        assert_eq!(hook.desc.as_deref(), Some("./check.sh"));

        let servers = &named(&groups, "MCP servers").items;
        assert_eq!(servers.len(), 2);
        assert_eq!(fact_of(&servers[0], "Host"), Some("mcp.example.com"));
        assert_eq!(fact_of(&servers[1], "Runs"), Some("serve"));
        assert_eq!(fact_of(&servers[1], "Environment"), Some("TOKEN"));

        let memory = &named(&groups, "Memory").items;
        assert_eq!(memory.len(), 1, "the index is not a memory");
        assert_eq!(memory[0].name, "merge-gate");
        assert_eq!(fact_of(&memory[0], "Modified"), Some("2026-11-27"));

        let shown = format!("{groups:?}");
        for secret in ["s3cret", "key=abc", "user:pw", "\"x\""] {
            assert!(!shown.contains(secret), "{secret} must not be shown");
        }
        assert_eq!(roots.first_scope(), Scope::Project);
    }

    #[test]
    fn an_unconfigured_project_shows_empty_kinds_and_opens_on_global() {
        let f = Fixture::new("bare");
        std::fs::create_dir_all(f.0.join("repo")).unwrap();
        let roots = f.roots();
        let groups = scan(Scope::Project, &roots);
        assert_eq!(groups.len(), 8);
        assert!(groups.iter().all(|g| g.items.is_empty()));
        assert_eq!(roots.first_scope(), Scope::Global);
    }

    #[test]
    fn global_scope_reads_plugins_and_user_servers() {
        let f = Fixture::new("global");
        f.write("home/.claude/CLAUDE.md", "");
        f.write(
            "home/.claude/settings.json",
            r#"{"model":"opus","enabledPlugins":{"frontend-design@official":true,"old@local":false},"statusLine":{"type":"command"}}"#,
        );
        f.write("home/.claude/settings.json.bak-statusline", "{}");
        f.write(
            "home/.claude.json",
            r#"{"mcpServers":{"github":{"command":"gh-mcp"}}}"#,
        );
        let groups = scan(Scope::Global, &f.roots());
        let claude = &named(&groups, "Instructions").items[0];
        assert_eq!(claude.hint, "empty");
        assert_eq!(fact_of(claude, "Size"), Some("empty"));
        let settings = &named(&groups, "Settings").items[0];
        assert_eq!(fact_of(settings, "Plugins"), Some("1 enabled"));
        assert_eq!(fact_of(settings, "Status line"), Some("custom"));
        assert_eq!(fact_of(settings, "Backups"), Some("1 hidden"));
        let plugins = &named(&groups, "Plugins").items;
        assert_eq!(plugins[0].name, "frontend-design");
        assert_eq!(plugins[0].hint, "official");
        assert_eq!(plugins[1].hint, "disabled");
        assert_eq!(named(&groups, "MCP servers").items[0].name, "github");
        assert!(named(&groups, "Hooks").items.is_empty());
    }

    #[test]
    fn tools_scope_summarises_the_configs_it_finds_and_hides_backups() {
        let f = Fixture::new("tools");
        f.write(
            "home/.config/herdr/config.toml",
            "[theme]\nname = \"gruvbox\"\n[theme.custom]\naccent = \"#a3b8a6\"\n[keys]\nprefix = \"ctrl+b\"\n[[keys.command]]\nkey = \"ctrl+e\"\n[[keys.command]]\nkey = \"ctrl+s\"\n",
        );
        f.write("home/.config/herdr/config.toml.bak", "");
        f.write("home/.config/herdr/config.toml.before-glass-20261105", "");
        f.write(
            "home/.config/herdr/plugins.json",
            r#"[{"name":"codeMap"},{"name":"paneMorph"}]"#,
        );
        f.write(
            "home/.config/yazi/yazi.toml",
            "[opener]\nbrowser = []\nnvim = []\n[open]\nprepend_rules = [{ url = \"*.html\", use = \"browser\" }]\n",
        );
        f.write(
            "home/.config/yazi/keymap.toml",
            "[[mgr.prepend_keymap]]\non = \"e\"\nrun = \"x\"\n",
        );
        f.write("home/.config/yazi/plugins/git.yazi/main.lua", "");
        f.write(
            "home/.config/ghostty/config",
            "theme = Catppuccin Mocha\nfont-family = JetBrainsMono\nfont-size = 16\n",
        );
        f.write(
            "home/.config/nvim/init.lua",
            "-- ====\n-- tuned for watching an agent\nvim.g.mapleader = \" \"\n",
        );
        f.write("home/.zshrc", "export TOKEN=abc\nalias g=git\n");
        let groups = scan(Scope::Tools, &f.roots());
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["herdr", "yazi", "neovim", "ghostty", "shell"]);

        let herdr = &named(&groups, "herdr").items;
        assert_eq!(herdr.len(), 2, "backup copies are not listed");
        assert_eq!(
            fact_of(&herdr[0], "Theme"),
            Some("gruvbox, custom accent #a3b8a6")
        );
        assert_eq!(fact_of(&herdr[0], "Key commands"), Some("2"));
        assert_eq!(fact_of(&herdr[0], "Backups"), Some("2 hidden"));
        assert_eq!(fact_of(&herdr[1], "Plugins"), Some("codeMap, paneMorph"));

        let yazi = &named(&groups, "yazi").items;
        assert_eq!(fact_of(&yazi[0], "Openers"), Some("browser, nvim"));
        assert_eq!(yazi[1].hint, "1 keys");
        let plugins = yazi.last().unwrap();
        assert!(plugins.dir);
        assert_eq!(fact_of(plugins, "Plugins"), Some("git"));

        assert_eq!(named(&groups, "ghostty").items[0].hint, "Catppuccin Mocha");
        assert_eq!(
            named(&groups, "neovim").items[0].desc.as_deref(),
            Some("tuned for watching an agent")
        );
        let shell = &named(&groups, "shell").items[0];
        assert!(shell.excerpt.is_empty() && shell.desc.is_none());
        assert!(
            !format!("{groups:?}").contains("abc"),
            "shell files are never quoted"
        );
    }

    #[test]
    fn other_agents_scope_finds_repository_and_home_files() {
        let f = Fixture::new("agents");
        f.write("repo/AGENTS.md", "# Agents\n\nRun the tests first.\n");
        f.write("repo/.cursor/rules/style.mdc", "Prefer small functions.\n");
        f.write("home/.codex/AGENTS.md", "Be brief.\n");
        f.write("home/.codex/config.toml", "model = \"gpt-5\"\napproval_policy = \"on-request\"\n[mcp_servers.docs]\ncommand = \"x\"\n");
        f.write("home/.codex/config.toml.before-statusline-1", "");
        let groups = scan(Scope::Agents, &f.roots());
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["In this repository", "Codex"]);
        let here: Vec<&str> = groups[0].items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(here, ["AGENTS.md", ".cursor/rules/style.mdc"]);
        assert_eq!(
            groups[0].items[0].desc.as_deref(),
            Some("Run the tests first.")
        );
        let config = &groups[1].items[1];
        assert_eq!(fact_of(config, "Model"), Some("gpt-5"));
        assert_eq!(fact_of(config, "MCP servers"), Some("1"));
        assert_eq!(fact_of(config, "Backups"), Some("1 hidden"));
    }

    #[test]
    fn backup_names_are_recognised() {
        for name in [
            "config.toml.bak",
            "init.lua.bak-yazi",
            "config.before-clear-glass-20261105",
            "config.backup-20261106",
            "notes.md~",
            "settings.json.orig",
        ] {
            assert!(is_backup(name), "{name}");
        }
        for name in ["config.toml", "backup.sh", "before.md"] {
            assert!(!is_backup(name), "{name}");
        }
        assert_eq!(
            host_of("https://user:pw@mcp.example.com/v1?key=abc"),
            "mcp.example.com"
        );
        assert_eq!(
            project_key(Path::new("/Users/dev/My Code/app")),
            "-Users-dev-My-Code-app"
        );
    }
}
