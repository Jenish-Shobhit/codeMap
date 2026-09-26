//! A tiny NDJSON client for herdr's socket API (protocol 22, herdr 0.9.0).
//!
//! One request per line, one response per line with the same `id`. Event
//! subscriptions keep their connection open and push one event per line.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for HerdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HerdrError {}

impl HerdrError {
    fn io(err: impl std::fmt::Display) -> Self {
        HerdrError {
            code: "io".to_string(),
            message: err.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    pub socket: PathBuf,
    pub timeout: Duration,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Client {
            socket: socket.into(),
            timeout: Duration::from_secs(5),
        }
    }

    /// The client for the herdr server that launched us, if any.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("HERDR_SOCKET_PATH")?;
        if path.is_empty() {
            return None;
        }
        Some(Client::new(PathBuf::from(path)))
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    fn connect(&self) -> Result<UnixStream, HerdrError> {
        let stream = UnixStream::connect(&self.socket).map_err(HerdrError::io)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(HerdrError::io)?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(HerdrError::io)?;
        Ok(stream)
    }

    /// Send one request and return its `result` object.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        let id = format!("codemorph-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let request = json!({ "id": id, "method": method, "params": params });
        let mut stream = self.connect()?;
        let mut line = serde_json::to_string(&request).map_err(HerdrError::io)?;
        line.push('\n');
        stream.write_all(line.as_bytes()).map_err(HerdrError::io)?;
        stream.flush().map_err(HerdrError::io)?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut response = String::new();
            let n = reader.read_line(&mut response).map_err(HerdrError::io)?;
            if n == 0 {
                return Err(HerdrError {
                    code: "closed".to_string(),
                    message: "herdr closed the connection".to_string(),
                });
            }
            let value: Value = match serde_json::from_str(response.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if value.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            return parse_response(value);
        }
    }

    /// Open an event subscription. Events arrive on the returned channel until
    /// the connection closes. The first acknowledgement is consumed here.
    pub fn subscribe(&self, subscriptions: Value) -> Result<mpsc::Receiver<Value>, HerdrError> {
        let id = format!("codemorph-sub-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let request = json!({
            "id": id,
            "method": "events.subscribe",
            "params": { "subscriptions": subscriptions },
        });
        let mut stream = self.connect()?;
        let mut line = serde_json::to_string(&request).map_err(HerdrError::io)?;
        line.push('\n');
        stream.write_all(line.as_bytes()).map_err(HerdrError::io)?;
        let mut reader = BufReader::new(stream);
        let mut ack = String::new();
        reader.read_line(&mut ack).map_err(HerdrError::io)?;
        let ack: Value = serde_json::from_str(ack.trim()).map_err(HerdrError::io)?;
        parse_response(ack)?;
        // Events can be far apart: no read timeout on the stream from here on.
        let _ = reader.get_ref().set_read_timeout(None);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = reader;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                            if tx.send(value).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });
        Ok(rx)
    }

    // ---- typed helpers --------------------------------------------------

    pub fn ping(&self) -> Result<Value, HerdrError> {
        self.call("ping", json!({}))
    }

    pub fn pane_get(&self, pane_id: &str) -> Result<Value, HerdrError> {
        let result = self.call("pane.get", json!({ "pane_id": pane_id }))?;
        Ok(result.get("pane").cloned().unwrap_or(result))
    }

    pub fn agent_list(&self) -> Result<Vec<Value>, HerdrError> {
        let result = self.call("agent.list", json!({}))?;
        Ok(result
            .get("agents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub fn worktree_list(&self, cwd: &str) -> Result<Vec<Value>, HerdrError> {
        let result = self.call("worktree.list", json!({ "cwd": cwd }))?;
        Ok(result
            .get("worktrees")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Type text into a pane without pressing Enter.
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        self.call("pane.send_text", json!({ "pane_id": pane_id, "text": text }))
            .map(|_| ())
    }

    /// Submit a prompt to an agent (text plus Enter, bracketed-paste aware).
    pub fn agent_prompt(&self, target: &str, text: &str) -> Result<(), HerdrError> {
        self.call("agent.prompt", json!({ "target": target, "text": text }))
            .map(|_| ())
    }

    pub fn pane_focus(&self, pane_id: &str) -> Result<(), HerdrError> {
        self.call("pane.focus", json!({ "pane_id": pane_id }))
            .map(|_| ())
    }

    pub fn notify(&self, title: &str, body: &str) -> Result<(), HerdrError> {
        let title: String = title.chars().take(80).collect();
        let body: String = body.chars().take(240).collect();
        self.call("notification.show", json!({ "title": title, "body": body }))
            .map(|_| ())
    }

    /// Open codeMorph's UI pane entrypoint.
    pub fn plugin_pane_open(&self, request: &PaneOpen) -> Result<Value, HerdrError> {
        self.call("plugin.pane.open", request.params())
    }
}

/// Parameters for `plugin.pane.open`.
#[derive(Debug, Clone)]
pub struct PaneOpen {
    pub plugin_id: String,
    pub entrypoint: String,
    pub placement: String,
    pub width: Option<String>,
    pub height: Option<String>,
    pub target_pane_id: Option<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub focus: bool,
}

impl PaneOpen {
    pub fn params(&self) -> Value {
        let mut params = json!({
            "plugin_id": self.plugin_id,
            "entrypoint": self.entrypoint,
            "placement": self.placement,
            "focus": self.focus,
        });
        let obj = params.as_object_mut().expect("object");
        if self.placement == "popup" {
            if let Some(w) = &self.width {
                obj.insert("width".into(), json!(w));
            }
            if let Some(h) = &self.height {
                obj.insert("height".into(), json!(h));
            }
        }
        if matches!(self.placement.as_str(), "split" | "zoomed") {
            if let Some(target) = &self.target_pane_id {
                obj.insert("target_pane_id".into(), json!(target));
            }
        }
        if let Some(cwd) = &self.cwd {
            obj.insert("cwd".into(), json!(cwd));
        }
        if !self.env.is_empty() {
            let env: serde_json::Map<String, Value> = self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            obj.insert("env".into(), Value::Object(env));
        }
        params
    }
}

fn parse_response(value: Value) -> Result<Value, HerdrError> {
    if let Some(err) = value.get("error") {
        return Err(HerdrError {
            code: err
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// A fake herdr: answers each request line with a canned result and
    /// records what it received.
    fn fake_server(dir: &Path, reply: Value) -> (PathBuf, mpsc::Receiver<Value>) {
        let path = dir.join("fake.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let request: Value = serde_json::from_str(line.trim()).unwrap();
                let id = request["id"].clone();
                tx.send(request).unwrap();
                let mut response = if reply.get("error").is_some() {
                    json!({ "id": id, "error": reply["error"] })
                } else {
                    json!({ "id": id, "result": reply })
                }
                .to_string();
                response.push('\n');
                let mut stream = stream;
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (path, rx)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cm-client-{name}-{}-{}",
            std::process::id(),
            crate::util::now_unix_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn call_round_trips_one_line() {
        let dir = temp_dir("rt");
        let (sock, seen) = fake_server(&dir, json!({ "type": "pong", "version": "0.9.0", "protocol": 22 }));
        let client = Client::new(&sock);
        let result = client.ping().unwrap();
        assert_eq!(result["protocol"], 22);
        let request = seen.recv().unwrap();
        assert_eq!(request["method"], "ping");
        assert!(request["id"].as_str().unwrap().starts_with("codemorph-"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_carry_code() {
        let dir = temp_dir("err");
        let (sock, _seen) = fake_server(
            &dir,
            json!({ "error": { "code": "agent_blocked", "message": "agent is blocked" } }),
        );
        let client = Client::new(&sock);
        let err = client.agent_prompt("w1:p1", "hi").unwrap_err();
        assert_eq!(err.code, "agent_blocked");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pane_open_params_match_schema() {
        let open = PaneOpen {
            plugin_id: "dev.codemorph".into(),
            entrypoint: "ui".into(),
            placement: "popup".into(),
            width: Some("94%".into()),
            height: Some("92%".into()),
            target_pane_id: Some("w1:p1".into()),
            cwd: None,
            env: vec![("CODEMORPH_VIEW".into(), "map".into())],
            focus: true,
        };
        let p = open.params();
        assert_eq!(p["width"], "94%");
        assert_eq!(p["height"], "92%");
        // popups target the active pane; herdr rejects target_pane_id there
        assert!(p.get("target_pane_id").is_none());
        assert_eq!(p["env"]["CODEMORPH_VIEW"], "map");

        let split = PaneOpen {
            placement: "split".into(),
            ..open
        };
        let p = split.params();
        assert!(p.get("width").is_none());
        assert_eq!(p["target_pane_id"], "w1:p1");
    }
}
