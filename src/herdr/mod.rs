pub mod client;
pub mod context;

pub use client::{Client, HerdrError, PaneOpen};
pub use context::{AgentStatusEvent, PluginContext, WorktreeCtx};

pub const PLUGIN_ID: &str = "dev.codemap";

pub fn inside_herdr() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|v| v == "1")
}

pub fn paste_payload(text: &str) -> String {
    if text.contains('\n') {
        format!("\x1b[200~{text}\x1b[201~")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_is_raw() {
        assert_eq!(paste_payload("hi"), "hi");
        assert_eq!(paste_payload("a\nb"), "\x1b[200~a\nb\x1b[201~");
    }
}
