# CosmicCut — instructions for agents

CosmicCut is CAM for XYB hot-wire foam cutters (wire in X and Y, B-axis turntable), built in
Rust from CosmicDMS (`../CosmicCAM`). Keep its look and its rules.

## Rules carried over from CosmicDMS
- **Never crash.** No `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!` or `unsafe` in
  non-test code (the crate roots deny them). Errors are `EngineResult`. Input-derived numbers
  are hostile: `get()` not `[i]`, finite checks, caps on sizes and counts.
  `Session::execute` restores the session if a command fails or panics.
- **Everything is a command** (`crates/engine/src/cmd/*`): the ribbon, explorer, console and
  tests reach the same `CommandSpec`s. Programmatic calls never open dialogs; only clicks with no
  parameters do (`app.rs` `run`, `tools.rs`, `cut_panel.rs`).
- **The UI is thin** and swappable: panes read the session and act through requests. Colours
  come from `theme::Tokens`; outer containers from `ui_kit`. Camera input only through
  `navigation::handle`.
- **Icons** are original SVG masters in `assets/icons/ui/design/svg/` on the 32 grid with colour
  roles (`assets/icons/ui/STYLE_GUIDE.md`). Every ribbon command needs one (a test checks).
  No artwork from commercial CAM products, ever. Assets go in `ATTRIBUTION.md`.
- Units inside are millimetres, degrees and seconds; the display units only change what is
  shown and typed.

## The model
- Table frame: origin on the turntable axis at the table top, Z up, wire along Y at B = 0.
  A wire position is `(u, h, b)` (`wire.rs`).
- A cut (`cut.rs`) gives a `WirePath` and the regions it keeps (`Keep`); the program
  (`program.rs`) joins the cuts; the simulation (`sim.rs`) and the post (`post.rs`) both play
  the program, so they always agree.
- Sketches (`sketch.rs`) are shapes with a `Sketch` attached: shared points, curves, and
  constraints held by a damped least-squares solver; snapping and automatic constraints are
  inferred in the engine (`Sketch::infer`), so mouse and console agree. The viewport tool
  (`viewport/sketch.rs`) only turns clicks into `sketch.*` commands.
- The table only turns with the wire clear of the foam: above it, or out past its turning circle
  (`cut::beside`, `cut::travel`, `program::link`). Beside the foam on the same side, the wire
  stays there between passes rather than climbing over the stock.

## Before finishing
`cargo fmt --all`, `cargo clippy --workspace --all-targets` (no warnings),
`cargo test --workspace`. Check UI changes with
`cargo run -- --run '<command>' --screenshot out.png`.
