//! codeMorph: see your code as a map, and what your agents changed on it.
//!
//! One binary, four views (Map, Flow, Changes, History), shipped as the herdr
//! plugin `dev.codemorph` and runnable on its own as `codemorph [path]`.

pub mod util;
pub mod herdr;
pub mod git;
pub mod store;
pub mod lang;
pub mod index;
pub mod canvas;
pub mod layout;
pub mod theme;
pub mod map;
pub mod flow;
