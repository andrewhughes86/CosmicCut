//! CosmicCut desktop application: CAM for XYB hot-wire foam cutters.
// A release build on Windows is a windowed app: no console window behind it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![allow(non_snake_case)]

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::process::ExitCode;

#[cfg(not(target_arch = "wasm32"))]
use CosmicCut_ui_egui::{Automation, LaunchOptions, Session, parse_console_line};

#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str =
    "usage: CosmicCut [job.ccut] [--empty] [--run \"<command> [json]\"]... [--tab <name>] [--screenshot <file.png>]

  job.ccut              open this job file
  --empty               start with an empty job instead of the sample
  --run <line>          run a command after start-up, e.g. --run 'view.theme.set {\"theme\":\"light\"}'
  --tab <name>          open on this ribbon tab (Home, Shapes, Cuts, Simulate, Machine)
  --screenshot <file>   save a PNG of the window once the steps have run, then quit";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut empty = false;
    let mut opened = false;
    let mut steps = Vec::new();
    let mut screenshot = None;
    let mut tab = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--empty" => empty = true,
            "--run" => match args.next().map(|line| parse_console_line(&line)) {
                Some(Ok(Some(step))) => steps.push(step),
                Some(Ok(None)) => {}
                Some(Err(e)) => return usage_error(&format!("--run: {e}")),
                None => return usage_error("--run needs a command"),
            },
            "--tab" => match args.next() {
                Some(name) => tab = Some(name),
                None => return usage_error("--tab needs a ribbon tab name"),
            },
            "--screenshot" => match args.next() {
                Some(path) => screenshot = Some(PathBuf::from(path)),
                None => return usage_error("--screenshot needs a file path"),
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            path if !path.starts_with('-') => {
                opened = true;
                steps.insert(0, ("job.open".to_owned(), serde_json::json!({ "path": path })));
            }
            other => return usage_error(&format!("unknown argument: {other}")),
        }
    }
    // Until there is a job, the sample shows what the program does.
    let session = if empty || opened { Session::new() } else { Session::with_sample() };
    // A screenshot run is a test: it must not change the operator's saved settings.
    let ephemeral = screenshot.is_some();
    let mut automation = Automation::new(steps, screenshot);
    automation.ribbon_tab = tab;
    match CosmicCut_ui_egui::run(session, LaunchOptions { automation, ephemeral }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("CosmicCut could not start: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn usage_error(message: &str) -> ExitCode {
    eprintln!("{message}\n\n{USAGE}");
    ExitCode::from(2)
}

/// In a browser: the sample job on the page's canvas, saving settings in local storage.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;
    let _ = eframe::WebLogger::init(log::LevelFilter::Warn);
    let document = web_sys::window().and_then(|w| w.document());
    let Some(canvas) = document
        .as_ref()
        .and_then(|d| d.get_element_by_id("cosmiccut"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        log::error!("CosmicCut: the page has no <canvas id=\"cosmiccut\">");
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let session = CosmicCut_ui_egui::Session::with_sample();
        let started = CosmicCut_ui_egui::run_web(canvas, session, CosmicCut_ui_egui::LaunchOptions::default()).await;
        // Swap the loading text for the app, or say why it could not start.
        let status = document.as_ref().and_then(|d| d.get_element_by_id("status"));
        match (started, status) {
            (Ok(()), Some(status)) => status.remove(),
            (Err(e), Some(status)) => status.set_text_content(Some(&format!(
                "CosmicCut could not start: {e:?}. It needs a browser with WebGPU or WebGL 2."
            ))),
            (Err(e), None) => log::error!("CosmicCut could not start: {e:?}"),
            (Ok(()), None) => {}
        }
    });
}
