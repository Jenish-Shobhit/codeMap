//! codeMap: see your code as a map, and what your agents changed on it.
//!
//! One binary, four views (Map, Flow, Changes, History), shipped as the herdr
//! plugin `dev.codemap` and runnable on its own as `codemap [path]`.

pub mod app;
pub mod canvas;
pub mod flow;
pub mod git;
pub mod herdr;
pub mod hook;
pub mod index;
pub mod keys;
pub mod lang;
pub mod layout;
pub mod map;
pub mod run;
pub mod store;
pub mod theme;
pub mod ui;
pub mod util;
