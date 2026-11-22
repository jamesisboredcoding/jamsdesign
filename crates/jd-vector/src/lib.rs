//! The vector design document model behind Blobr.
//!
//! A [`Document`] is a tree of pages, frames, shapes and text ([`Node`]s). Every change goes
//! through [`Op`]s applied in an atomic batch with [`Document::apply`]: the GUI, undo/redo and
//! (later) the MCP server all use that one path. After each batch, auto layout and groups are
//! recomputed ([`layout`]).
//!
//! Nodes live behind `Arc`, so cloning a document is cheap. That is what makes batches atomic
//! (they run on a clone that only replaces the document on success) and what undo is built on
//! ([`History`] keeps whole-document snapshots, like `jd-raster`).
//!
//! No iced and no GPU here: rendering lives elsewhere, so documents can be edited and saved
//! without a window.

mod color;
pub mod component;
mod doc;
pub mod file;
pub mod geometry;
pub mod history;
pub mod layout;
pub mod node;
pub mod ops;
pub mod text;
pub mod tokens;

pub use doc::Document;
pub use history::{Editor, History};
pub use jd_core::Rgba;
pub use node::*;
pub use ops::{Applied, NodeSpec, Op, OpError, Patch, Ref};
pub use text::{ApproxMeasure, TextMeasure};
pub use tokens::{Collection, Style, StyleKind, Tokens, VarValue, Variable};
