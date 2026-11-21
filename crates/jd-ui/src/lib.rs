//! Shared look & feel for every jamsdesign app: colour tokens (mirroring the Figma
//! `jamsdesign/color` variables), icons, and the small set of widgets the panels are built from.

pub mod icon;
pub mod style;
pub mod theme;
pub mod widget;

pub use icon::{Icon, icon};
pub use theme::{theme, tokens};
