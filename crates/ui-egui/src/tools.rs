//! Interactive starts: what a ribbon click or shortcut does for commands that need input first.
//! Each one gathers that input (a file dialog or a form) and then runs the command with
//! explicit parameters; commands run from the console or scripts never get here.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use CosmicCut_engine::{Session, cmd};
use serde_json::{Value, json};

use crate::Request;
use crate::param_form::ParamForm;
use crate::theme::Tokens;

/// Commands whose `path` comes from a file dialog.
const FILE_COMMANDS: &[&str] =
    &["model.import", "shape.import", "job.open", "job.save", "job.save.as", "post.gcode.export"];

/// Commands started by a dialog generated from their parameters.
const FORM_COMMANDS: &[&str] = &[
    "stock.block",
    "stock.cylinder",
    "machine.edit",
    "shape.rectangle",
    "shape.circle",
    "shape.polygon",
    "shape.star",
    "shape.airfoil",
    "shape.transform",
    "model.place",
    "model.scale",
    "stock.fit",
];

/// Commands with an interactive start.
pub fn has_start(id: &str) -> bool {
    FORM_COMMANDS.contains(&id) || FILE_COMMANDS.contains(&id)
}

/// What a file dialog asks for: title, filter label, extensions, suggested name (saves).
struct FileKind {
    title: &'static str,
    filter: &'static str,
    extensions: &'static [&'static str],
    save_as: Option<&'static str>,
}

const SHAPE_IN: FileKind = FileKind {
    title: "Import shape",
    filter: "Drawings and coordinates (DXF, SVG, DAT, TXT, CSV)",
    extensions: &["dxf", "DXF", "svg", "SVG", "dat", "DAT", "txt", "TXT", "csv", "CSV", "xy", "XY"],
    save_as: None,
};
const JOB_IN: FileKind =
    FileKind { title: "Open job", filter: "CosmicCut job (.ccut)", extensions: &["ccut", "CCUT"], save_as: None };
const JOB_OUT: FileKind = FileKind {
    title: "Save job",
    filter: "CosmicCut job (.ccut)",
    extensions: &["ccut", "CCUT"],
    save_as: Some("job.ccut"),
};
const GCODE_OUT: FileKind = FileKind {
    title: "Post G-code",
    filter: "G-code program",
    extensions: &["ngc", "NGC", "nc", "NC", "tap", "gcode", "txt"],
    save_as: Some("program.ngc"),
};

/// An interactive start waiting for the user.
enum Pending {
    Form(Box<ParamForm>),
    /// A file dialog choosing `path` for a command.
    File(&'static str, Receiver<Option<PathBuf>>),
}

#[derive(Default)]
pub struct Tools {
    pending: Option<Pending>,
    #[cfg(target_arch = "wasm32")]
    drops: Drops,
}

/// Dropped files the browser has finished reading, as the commands that take them.
#[cfg(target_arch = "wasm32")]
struct Drops(std::sync::mpsc::Sender<Request>, Receiver<Request>);

#[cfg(target_arch = "wasm32")]
impl Default for Drops {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self(tx, rx)
    }
}

impl Tools {
    /// Starts `id`'s interactive tool, with `given` filling some of its values. Errors are
    /// user-facing messages.
    pub fn start(&mut self, id: &str, session: &Session, given: &Value) -> Result<(), String> {
        // A browser may never say its file picker was dismissed, so a new one replaces it.
        let stale = cfg!(target_arch = "wasm32") && self.is_busy();
        if self.pending.is_some() && !stale {
            return Err("finish the open dialog first".into());
        }
        let file = |kind: &FileKind, name: Option<String>| pick_file(kind, name);
        let base = |ext: &str| {
            let stem = if session.job.name.trim().is_empty() { "job" } else { session.job.name.trim() };
            Some(format!("{stem}.{ext}"))
        };
        let pending = match id {
            "shape.import" => Pending::File("shape.import", file(&SHAPE_IN, None)?),
            "model.import" => Pending::File("model.import", pick_model()?),
            "job.open" => Pending::File("job.open", file(&JOB_IN, None)?),
            "job.save" | "job.save.as" => Pending::File(
                if id == "job.save" { "job.save" } else { "job.save.as" },
                file(&JOB_OUT, base(CosmicCut_engine::jobfile::EXTENSION))?,
            ),
            "post.gcode.export" => {
                let ext = session.job.machine.post.extension.clone();
                Pending::File("post.gcode.export", file(&GCODE_OUT, base(&ext))?)
            }
            _ => {
                let spec = cmd::find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
                if !session.is_enabled(spec) {
                    return Err(format!("{} is not available right now", spec.label));
                }
                Pending::Form(Box::new(ParamForm::new(spec, session, given)))
            }
        };
        self.pending = Some(pending);
        Ok(())
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.pending, Some(Pending::File(..)))
    }

    /// Draws an open form. Returns the command to run when it is confirmed.
    pub fn show(&mut self, ctx: &egui::Context, tokens: &Tokens) -> Option<Request> {
        let Some(Pending::Form(form)) = &mut self.pending else { return None };
        let request = form.show(ctx, tokens);
        if !form.is_open() {
            self.pending = None;
        }
        request
    }

    /// The command run from the form finished: close it, or keep it open showing the error.
    pub fn finished(&mut self, outcome: Result<(), String>) {
        match (&mut self.pending, outcome) {
            (Some(Pending::Form(form)), Err(e)) => form.fail(e),
            (Some(Pending::Form(_)), Ok(())) => self.pending = None,
            _ => {}
        }
    }

    /// Commands whose file dialogs have closed.
    pub fn poll(&mut self) -> Vec<Request> {
        #[cfg(target_arch = "wasm32")]
        {
            let dropped: Vec<Request> = self.drops.1.try_iter().collect();
            if !dropped.is_empty() {
                return dropped;
            }
        }
        let Some(Pending::File(command, rx)) = &self.pending else { return Vec::new() };
        match rx.try_recv() {
            Ok(choice) => {
                let command = *command;
                self.pending = None;
                choice.map(|path| (command.to_owned(), json!({ "path": path.to_string_lossy() }))).into_iter().collect()
            }
            Err(TryRecvError::Empty) => Vec::new(),
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                Vec::new()
            }
        }
    }

    /// Files dropped on the window: jobs open, drawings are imported as shapes.
    pub fn dropped(&mut self, files: &[egui::DroppedFileHandle]) -> Vec<Request> {
        let requests = files.iter().map(|f| {
            let p = f.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
            // Drawings are shapes (a DXF too); any other model file is a model.
            let command = if ext == CosmicCut_engine::jobfile::EXTENSION {
                "job.open"
            } else if CosmicCut_io::profile::ProfileFormat::from_extension(&ext).is_some() {
                "shape.import"
            } else {
                "model.import"
            };
            (command.to_owned(), json!({ "path": p.to_string_lossy() }))
        });
        #[cfg(target_arch = "wasm32")]
        {
            // The browser reads each file later; its command runs once it has.
            for (file, request) in files.iter().zip(requests) {
                let (tx, file) = (self.drops.0.clone(), file.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(bytes) = file.bytes_async().await {
                        CosmicCut_io::files::provide(&file.path().to_string_lossy(), bytes);
                        let _ = tx.send(request);
                    }
                });
            }
            Vec::new()
        }
        #[cfg(not(target_arch = "wasm32"))]
        requests.collect()
    }
}

/// The model import dialog: every model format, then each on its own.
fn pick_model() -> Result<Receiver<Option<PathBuf>>, String> {
    use CosmicCut_io::ModelFormat;
    let both =
        |exts: &[&str]| -> Vec<String> { exts.iter().flat_map(|e| [e.to_string(), e.to_ascii_uppercase()]).collect() };
    let all: Vec<&str> = ModelFormat::ALL.iter().flat_map(|f| f.extensions().iter().copied()).collect();
    #[cfg(target_arch = "wasm32")]
    {
        let dialog = rfd::AsyncFileDialog::new().set_title("Import model").add_filter("All models", &both(&all));
        Ok(upload(dialog))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut dialog = rfd::FileDialog::new().set_title("Import model").add_filter("All models", &both(&all));
        for f in ModelFormat::ALL {
            dialog = dialog.add_filter(f.label(), &both(f.extensions()));
        }
        native_pick(move || dialog.pick_file())
    }
}

/// Runs a blocking dialog where it can block; the chosen path arrives on the channel.
#[cfg(not(target_arch = "wasm32"))]
fn native_pick(pick: impl FnOnce() -> Option<PathBuf> + Send + 'static) -> Result<Receiver<Option<PathBuf>>, String> {
    let (tx, rx) = channel();
    #[cfg(target_os = "macos")]
    {
        let _ = tx.send(pick());
    }
    // macOS dialogs must run on the main thread; elsewhere the dialog gets its own so the
    // window keeps drawing.
    #[cfg(not(target_os = "macos"))]
    std::thread::Builder::new()
        .name("file-dialog".into())
        .spawn(move || {
            let _ = tx.send(pick());
        })
        .map_err(|e| format!("could not open the file dialog: {e}"))?;
    Ok(rx)
}

/// The browser's file picker: the chosen file is read into memory and its name arrives on the
/// channel, for the command to read back through `CosmicCut_io::files`.
#[cfg(target_arch = "wasm32")]
fn upload(dialog: rfd::AsyncFileDialog) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = channel();
    wasm_bindgen_futures::spawn_local(async move {
        let chosen = match dialog.pick_file().await {
            Some(file) => {
                let name = file.file_name();
                CosmicCut_io::files::provide(&name, file.read().await);
                Some(PathBuf::from(name))
            }
            None => None,
        };
        let _ = tx.send(chosen);
    });
    rx
}

/// An open or save dialog; the chosen path arrives on the channel. `name` overrides the
/// suggested file name of a save dialog.
fn pick_file(kind: &FileKind, name: Option<String>) -> Result<Receiver<Option<PathBuf>>, String> {
    let save = kind.save_as.is_some();
    let name = name.as_deref().or(kind.save_as).filter(|_| save).map(str::to_owned);
    #[cfg(target_arch = "wasm32")]
    {
        // A save in a browser is a download of the suggested name: no dialog.
        if let Some(name) = name {
            let (tx, rx) = channel();
            let _ = tx.send(Some(PathBuf::from(name)));
            return Ok(rx);
        }
        Ok(upload(rfd::AsyncFileDialog::new().set_title(kind.title).add_filter(kind.filter, kind.extensions)))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dialog = rfd::FileDialog::new().set_title(kind.title).add_filter(kind.filter, kind.extensions);
        let dialog = match name {
            Some(name) => dialog.set_file_name(name),
            None => dialog,
        };
        native_pick(move || if save { dialog.save_file() } else { dialog.pick_file() })
    }
}
