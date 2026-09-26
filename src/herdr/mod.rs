//! Everything codeMorph says to herdr goes through this module.

pub mod client;
pub mod context;

pub use client::{Client, HerdrError, PaneOpen};
pub use context::{AgentStatusEvent, PluginContext, WorktreeCtx};

/// The plugin id declared in `herdr-plugin.toml`.
pub const PLUGIN_ID: &str = "dev.codemorph";

/// True when we were started by herdr (plugin action, pane or hook).
pub fn inside_herdr() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|v| v == "1")
}

/// Wrap multi-line text in bracketed-paste markers so an agent prompt (or a
/// shell) receives it as one paste instead of submitting at each newline.
/// `pane.send_text` writes raw bytes, so codeMorph adds the markers itself.
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
