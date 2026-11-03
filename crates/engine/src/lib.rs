//! L4 engine for CosmicCut, a CAM program for hot-wire foam cutters: the session, its commands
//! and the cutting model behind them.
//!
//! The machine moves a heated wire in X (across) and Y (up) over a B-axis turntable. Inside,
//! everything is millimetres, degrees and seconds, in the *table frame*: the origin is the
//! turntable's axis on the table top, Z is up, and at B = 0 the wire runs along Y. A wire
//! position is `(u, h, b)`: `u` the wire's offset across the table from the axis (machine X
//! less the table's X), `h` its height above the table top, `b` the turntable angle.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![allow(non_snake_case)]

pub mod cmd;
pub mod cut;
pub mod error;
pub mod interference;
pub mod jobfile;
pub mod library;
pub mod machine;
pub mod model;
pub mod op;
pub mod params;
pub mod post;
pub mod program;
pub mod session;
pub mod shape;
pub mod sim;
pub mod sketch;
pub mod stock;
pub mod swarf;
pub mod units;
pub mod wire;

pub use cmd::{ButtonSize, CommandSpec, Param, ParamKind, Placement, RibbonTab};
pub use error::{EngineError, EngineResult, error_json};
pub use session::{Display, Pane, Session, Theme, UiRequest};

#[cfg(test)]
mod tests;
