//! The post-processor: the program as RS274/NGC (LinuxCNC-style) G-code for an XYB hot-wire
//! machine, written the way [`PostSettings`] says: the user's start (safety) block and end code,
//! the codes that switch the wire on and off, whether every move line carries its G word, line
//! numbers, decimals and more.

use std::fmt::Write as _;

use CosmicCut_geom::arcfit::{self, ArcLimits, Segment};
use glam::DVec3;
use serde_json::{Value, json};

use crate::cut::{Cut, fmt_time};
use crate::error::{EngineError, EngineResult};
use crate::machine::{Machine, Origin};
use crate::op::Operation;
use crate::params;
use crate::program::Program;
use crate::stock::Stock;
use crate::units::DisplayUnits;
use crate::wire::MoveKind;

/// How programs are written. Code blocks are plain text, one G-code line per text line; in
/// them `{units}` becomes G20 or G21, `{job}` the job's name and `{preheat}` the preheat
/// seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct PostSettings {
    /// Units programs are written in.
    pub units: DisplayUnits,
    /// The start (safety) block, after the program's opening comments.
    pub header: String,
    /// Switches the wire on, before the first move into foam.
    pub wire_on: String,
    /// Switches the wire off, at the end and (with `wire_off_between_cuts`) between cuts.
    pub wire_off: String,
    /// Seconds the wire heats after it is switched on (a dwell); 0 for none.
    pub preheat: f64,
    /// Switch the wire off while it travels between cuts, and on (with the preheat) again.
    pub wire_off_between_cuts: bool,
    /// Written between cuts, with the wire above the foam (e.g. `M0` to pause).
    pub between_cuts: String,
    /// The end code, after the wire is switched off.
    pub footer: String,
    /// A G0 or G1 on every move line, not only when it changes.
    pub motion_every_line: bool,
    /// The feed on every feed move, not only when it changes.
    pub feed_every_line: bool,
    /// Write G2/G3 arcs where the path follows circles.
    pub arcs: bool,
    /// The letter of the turntable axis.
    pub b_letter: String,
    /// Turn the table with G1 at `b_feed` instead of G0, for controllers that will not rapid B.
    pub b_g1: bool,
    /// Feed of those moves, written as the F word as it is (degrees per minute on most
    /// controllers).
    pub b_feed: f64,
    /// Decimal places of X and Y; none for 4 in inches, 3 in millimetres.
    pub decimals: Option<u32>,
    /// N numbers on each line.
    pub line_numbers: bool,
    pub line_start: u32,
    pub line_step: u32,
    /// Comments naming the job, the stock and each cut.
    pub comments: bool,
    /// `%` at the start and end.
    pub percent: bool,
    /// File extension programs are saved with.
    pub extension: String,
    /// End by going home to X0 Y0, over the top of the travel when the part is in the way.
    pub return_home: bool,
    /// After the last cut, turn the table to B0 or B180, whichever is nearer, before going home;
    /// false leaves it where the last cut finished.
    pub square_b: bool,
}

impl Default for PostSettings {
    /// Suits the HotwireCNC controller (LinuxCNC dialect) in inches.
    fn default() -> Self {
        Self {
            units: DisplayUnits::Inches,
            header: "G17 G90 G91.1 G40 G49 G94\n{units}".into(),
            wire_on: "M3".into(),
            wire_off: "M5".into(),
            preheat: 2.0,
            wire_off_between_cuts: false,
            between_cuts: String::new(),
            footer: "M2".into(),
            motion_every_line: false,
            feed_every_line: false,
            arcs: true,
            b_letter: "B".into(),
            b_g1: false,
            b_feed: 1800.0,
            decimals: None,
            line_numbers: false,
            line_start: 10,
            line_step: 10,
            comments: true,
            percent: true,
            extension: "ngc".into(),
            return_home: true,
            square_b: true,
        }
    }
}

/// `post.settings.set` parameters, in the order the dialog shows them.
pub const PARAMS: &[crate::Param] = {
    use crate::ParamKind::*;
    use crate::cmd::Param;
    &[
        Param::optional("name", Str, "The library post to change (default the machine's)"),
        Param::optional("units", Choice(&["in", "mm"]), "Units programs are written in"),
        Param::optional("header", Str, "Start (safety) block; {units} becomes G20 or G21"),
        Param::optional("wire_on", Str, "Code that switches the wire on (e.g. M3, or M3 S60)"),
        Param::optional("wire_off", Str, "Code that switches the wire off (e.g. M5)"),
        Param::optional("preheat", Number { min: 0.0, max: 600.0 }, "Seconds the wire heats after it is switched on"),
        Param::optional("wire_off_between_cuts", Bool, "Switch the wire off while it travels between cuts"),
        Param::optional("between_cuts", Str, "Code written between cuts, e.g. M0 to pause"),
        Param::optional("footer", Str, "End code, after the wire is switched off"),
        Param::optional("motion_every_line", Bool, "A G0 or G1 on every move line"),
        Param::optional("feed_every_line", Bool, "The feed on every feed move"),
        Param::optional("arcs", Bool, "Write G2/G3 arcs where the path follows circles"),
        Param::optional("b_letter", Str, "Letter of the turntable axis"),
        Param::optional("b_g1", Bool, "Turn the table with G1 at the B feed instead of G0"),
        Param::optional("b_feed", Number { min: 0.001, max: 1.0e6 }, "Feed of G1 table turns, the F word as written"),
        Param::optional("decimals", Integer { min: 0, max: 8 }, "Decimal places of X and Y (default 4 in, 3 mm)"),
        Param::optional("line_numbers", Bool, "N numbers on each line"),
        Param::optional("line_start", Integer { min: 0, max: 1_000_000 }, "First line number"),
        Param::optional("line_step", Integer { min: 1, max: 1000 }, "Line number step"),
        Param::optional("comments", Bool, "Comments naming the job, the stock and each cut"),
        Param::optional("percent", Bool, "% at the start and end of the program"),
        Param::optional("extension", Str, "File extension programs are saved with"),
        Param::optional("return_home", Bool, "End by going home to X0 Y0, over the top when the part is in the way"),
        Param::optional("square_b", Bool, "After the last cut, turn the table to B0 or B180, whichever is nearer"),
    ]
};

impl PostSettings {
    pub fn to_json(&self) -> Value {
        json!({
            "units": self.units.id(),
            "header": self.header,
            "wire_on": self.wire_on,
            "wire_off": self.wire_off,
            "preheat": self.preheat,
            "wire_off_between_cuts": self.wire_off_between_cuts,
            "between_cuts": self.between_cuts,
            "footer": self.footer,
            "motion_every_line": self.motion_every_line,
            "feed_every_line": self.feed_every_line,
            "arcs": self.arcs,
            "b_letter": self.b_letter,
            "b_g1": self.b_g1,
            "b_feed": self.b_feed,
            "decimals": self.decimals,
            "line_numbers": self.line_numbers,
            "line_start": self.line_start,
            "line_step": self.line_step,
            "comments": self.comments,
            "percent": self.percent,
            "extension": self.extension,
            "return_home": self.return_home,
            "square_b": self.square_b,
        })
    }

    /// Applies the fields present in `p`; unknown fields (from newer files) are ignored.
    pub fn apply(&mut self, p: &Value) -> EngineResult<()> {
        let mut s = self.clone();
        let text = |k: &str| -> EngineResult<Option<String>> {
            Ok(params::opt_str(p, k)?.map(|t| t.replace("\r\n", "\n").chars().take(4000).collect()))
        };
        let flag = |k: &str| params::opt_bool(p, k);
        if let Some(u) = params::opt_str(p, "units")? {
            s.units = DisplayUnits::from_id(u)
                .ok_or_else(|| EngineError::InvalidParams("`units` must be in or mm".into()))?;
        }
        for (k, slot) in [
            ("header", &mut s.header),
            ("wire_on", &mut s.wire_on),
            ("wire_off", &mut s.wire_off),
            ("between_cuts", &mut s.between_cuts),
            ("footer", &mut s.footer),
        ] {
            if let Some(t) = text(k)? {
                *slot = t;
            }
        }
        for (k, slot) in [
            ("wire_off_between_cuts", &mut s.wire_off_between_cuts),
            ("motion_every_line", &mut s.motion_every_line),
            ("feed_every_line", &mut s.feed_every_line),
            ("arcs", &mut s.arcs),
            ("line_numbers", &mut s.line_numbers),
            ("comments", &mut s.comments),
            ("percent", &mut s.percent),
            ("return_home", &mut s.return_home),
            ("square_b", &mut s.square_b),
            ("b_g1", &mut s.b_g1),
        ] {
            if let Some(b) = flag(k)? {
                *slot = b;
            }
        }
        if let Some(v) = params::opt_f64(p, "preheat", 0.0, 600.0)? {
            s.preheat = v;
        }
        if let Some(v) = params::opt_f64(p, "b_feed", 0.001, 1.0e6)? {
            s.b_feed = v;
        }
        match p.get("decimals") {
            Some(Value::Null) => s.decimals = None,
            Some(_) => s.decimals = params::opt_f64(p, "decimals", 0.0, 8.0)?.map(|v| v as u32),
            None => {}
        }
        if let Some(v) = params::opt_f64(p, "line_start", 0.0, 1.0e6)? {
            s.line_start = v as u32;
        }
        if let Some(v) = params::opt_f64(p, "line_step", 1.0, 1000.0)? {
            s.line_step = v as u32;
        }
        if let Some(t) = text("b_letter")? {
            let t = t.trim().to_ascii_uppercase();
            if t.len() != 1 || !t.chars().all(|c| c.is_ascii_alphabetic()) || "XYGMFNSTPIJKR".contains(&t) {
                return Err(EngineError::InvalidParams(
                    "the turntable letter must be one free axis letter, e.g. B".into(),
                ));
            }
            s.b_letter = t;
        }
        if let Some(t) = text("extension")? {
            let t = t.trim().trim_start_matches('.').to_owned();
            if t.is_empty() || !t.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(EngineError::InvalidParams("the extension must be letters and digits, e.g. ngc".into()));
            }
            s.extension = t;
        }
        *self = s;
        Ok(())
    }
}

/// A posted program.
#[derive(Clone, Debug, Default)]
pub struct Posted {
    pub text: String,
    pub lines: usize,
    pub seconds: f64,
    /// Limits the program goes past, and other things to look at.
    pub warnings: Vec<String>,
}

impl Posted {
    pub fn summary_json(&self) -> Value {
        json!({ "lines": self.lines, "seconds": self.seconds, "warnings": self.warnings })
    }
}

/// The output as it is written, with its modal state so words are written only when they
/// change (unless the settings ask for them on every line).
struct Out<'a> {
    s: &'a PostSettings,
    text: String,
    next_n: u32,
    motion: Option<u8>,
    x: Option<String>,
    y: Option<String>,
    b: Option<String>,
    /// The feed last written, and one to write on the next feed move.
    f: Option<String>,
    pending_f: Option<String>,
}

impl Out<'_> {
    /// One line of code, numbered when asked.
    fn line(&mut self, s: &str) {
        let s = s.trim_end();
        if s.is_empty() {
            return;
        }
        if self.s.line_numbers && s != "%" && !s.starts_with('(') {
            let _ = write!(self.text, "N{} ", self.next_n);
            self.next_n = self.next_n.saturating_add(self.s.line_step.max(1));
        }
        self.text.push_str(s);
        self.text.push('\n');
    }

    /// A block of user code, one line per text line, with its tokens filled in.
    fn block(&mut self, code: &str, job: &str) {
        let units = if self.s.units == DisplayUnits::Inches { "G20" } else { "G21" };
        for line in code.lines() {
            let line = line
                .replace("{units}", units)
                .replace("{job}", &ascii(job))
                .replace("{preheat}", &trim(format!("{:.2}", self.s.preheat)));
            self.line(line.trim());
        }
    }

    fn comment(&mut self, s: &str) {
        if self.s.comments {
            let c = format!("({})", ascii(s));
            self.line(&c);
        }
    }

    fn wire_on(&mut self, job: &str) {
        let code = self.s.wire_on.clone();
        self.block(&code, job);
        if self.s.preheat > 0.0 {
            self.line(&format!("G4 P{}", trim(format!("{:.2}", self.s.preheat))));
        }
    }

    fn wire_off(&mut self, job: &str) {
        let code = self.s.wire_off.clone();
        self.block(&code, job);
    }

    fn set_feed(&mut self, feed: f64) {
        let u = self.s.units;
        let f = trim(format!("{:.*}", if u == DisplayUnits::Inches { 2 } else { 1 }, u.from_mm(feed)));
        self.pending_f = (self.s.feed_every_line || self.f.as_deref() != Some(f.as_str())).then_some(f);
    }

    /// The F word for a feed move, when it is due.
    fn take_feed(&mut self) -> String {
        match self.pending_f.take() {
            Some(f) => {
                let word = format!(" F{f}");
                if self.s.feed_every_line {
                    self.pending_f = Some(f.clone());
                }
                self.f = Some(f);
                word
            }
            None => String::new(),
        }
    }

    /// A G0/G1 with the words that changed (all of X, Y and the G word when asked for). A rapid
    /// that turns the table is a G1 at the B feed when the settings ask for it.
    fn linear(&mut self, g: u8, x: &str, y: &str, b: Option<String>) {
        let mut s = String::new();
        let turns = b.as_ref().is_some_and(|b| self.b.as_deref() != Some(b.as_str()));
        let moved = self.x.as_deref() != Some(x) || self.y.as_deref() != Some(y) || turns;
        if !moved {
            return;
        }
        let b_feed = (g == 0 && turns && self.s.b_g1).then(|| trim(format!("{:.2}", self.s.b_feed)));
        let g = if b_feed.is_some() { 1 } else { g };
        if self.s.motion_every_line || self.motion != Some(g) {
            let _ = write!(s, "G{g}");
            self.motion = Some(g);
        }
        if self.x.as_deref() != Some(x) {
            let _ = write!(s, " X{x}");
            self.x = Some(x.to_owned());
        }
        if self.y.as_deref() != Some(y) {
            let _ = write!(s, " Y{y}");
            self.y = Some(y.to_owned());
        }
        if let Some(b) = b
            && self.b.as_deref() != Some(b.as_str())
        {
            let _ = write!(s, " {}{b}", self.s.b_letter);
            self.b = Some(b);
        }
        if let Some(f) = b_feed {
            if self.s.feed_every_line || self.f.as_deref() != Some(f.as_str()) {
                let _ = write!(s, " F{f}");
            }
            self.f = Some(f);
        } else if g == 1 {
            s.push_str(&self.take_feed());
        }
        self.line(s.trim());
    }
}

/// The comments that open operation `op` (its place `n`): its title and notes, when the job
/// has more than one operation or the operation says something.
fn op_head(out: &mut Out<'_>, n: usize, op: &Operation, count: usize) {
    if count > 1 || !op.name.trim().is_empty() {
        out.comment(&op.title(n));
    }
    for line in op.notes.lines().map(str::trim).filter(|l| !l.is_empty()) {
        out.comment(line);
    }
}

/// The operation's message and machine stop.
fn op_stop(out: &mut Out<'_>, op: &Operation) {
    if !op.message.trim().is_empty() {
        // Always written, whatever the comment setting: the controller shows it.
        let msg = format!("(MSG, {})", ascii(op.message.trim()).replace('[', " "));
        out.line(&msg);
    }
    if let Some(code) = op.stop.code() {
        out.line(code);
    }
}

fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '(' | ')' => '[',
            '×' => 'x',
            'Ø' => 'D',
            '°' => 'd',
            c if c.is_ascii() && !c.is_ascii_control() => c,
            _ => '?',
        })
        .collect()
}

pub fn post(
    machine: &Machine,
    origin: Origin,
    stock: &Stock,
    cuts: &[Cut],
    ops: &[Operation],
    job_name: &str,
) -> Posted {
    let settings = &machine.post;
    let prog = Program::build(machine, stock, cuts, ops);
    let op_at = |i: usize| prog.op_starts.iter().find(|(at, _)| *at == i).and_then(|(_, o)| Some((*o, ops.get(*o)?)));
    let units = settings.units;
    let places = settings.decimals.unwrap_or(if units == DisplayUnits::Inches { 4 } else { 3 }) as usize;
    // Decimals the user set are written in full (trailing zeros too); the default trims them.
    let fixed = settings.decimals.is_some();
    let num = |mm: f64| {
        let s = format!("{:.*}", places, units.from_mm(mm));
        if !fixed {
            return trim(s);
        }
        // "-0.0000" reads as "0.0000".
        match s.strip_prefix('-') {
            Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_owned(),
            _ => s,
        }
    };
    let deg = |d: f64| trim(format!("{d:.3}"));
    let mut out = Out {
        s: settings,
        text: String::new(),
        next_n: settings.line_start,
        motion: None,
        x: None,
        y: None,
        b: None,
        f: None,
        pending_f: None,
    };
    let mut warnings = Vec::new();

    if settings.percent {
        out.line("%");
    }
    out.comment(&format!("CosmicCut program: {job_name}"));
    out.comment(&format!("Machine: {}", machine.name));
    out.comment(&format!("Origin: {}", origin.label()));
    out.comment(&format!("Stock: {}", stock.describe(units)));
    let active = cuts.iter().filter(|c| c.error.is_none() && !c.path.moves.is_empty()).count();
    out.comment(&format!("{active} cuts, about {}", fmt_time(prog.duration() + settings.preheat)));
    for c in cuts.iter().filter(|c| c.error.is_some()) {
        warnings.push(format!("{} was left out: {}", c.name, c.error.as_deref().unwrap_or_default()));
    }
    out.block(&settings.header, job_name);
    if prog.moves.is_empty() {
        out.comment("Nothing to cut");
        out.block(&settings.footer, job_name);
        if settings.percent {
            out.line("%");
        }
        let lines = out.text.lines().count();
        warnings.push("the job has no cuts to post".into());
        return Posted { text: out.text, lines, seconds: 0.0, warnings };
    }

    // The first operation's stop comes first: the operator loads the stock.
    if let Some((n, op)) = op_at(0) {
        op_head(&mut out, n, op, ops.len());
        op_stop(&mut out, op);
    }
    // From home to where the first cut begins (over the stock only if it is in the way), then
    // heat the wire there.
    for (_, m) in prog.moves.iter().take(prog.approach_end) {
        let (x, y) = machine.program_xy(origin, m.to.u, m.to.h);
        let b = machine.b_axis.then(|| deg(machine.machine_b(m.to.b)));
        out.linear(0, &num(x), &num(y), b);
    }
    if prog.approach_end == 0 {
        let (x0, y0) = machine.program_xy(origin, prog.start.u, prog.start.h);
        let b0 = machine.b_axis.then(|| deg(machine.machine_b(prog.start.b)));
        out.linear(0, &num(x0), &num(y0), b0);
    }
    out.wire_on(job_name);

    let mut current_cut: Option<usize> = None;
    let mut wire_hot = true;
    let mut i = prog.approach_end;
    let n = prog.moves.len();
    while i < n {
        let Some((cut, m)) = prog.moves.get(i).copied() else { break };
        // A later operation: the wire off while the machine stops, on again after.
        if i > 0
            && let Some((n, op)) = op_at(i)
        {
            op_head(&mut out, n, op, ops.len());
            if op.pauses() {
                if wire_hot {
                    out.wire_off(job_name);
                    wire_hot = false;
                }
                op_stop(&mut out, op);
            }
        }
        // Home: the wire off first, then clear of the part to X0 Y0.
        if prog.home_at == Some(i) {
            if wire_hot {
                out.wire_off(job_name);
                wire_hot = false;
            }
            out.comment(&match (prog.end_turn, machine.post.return_home) {
                (Some(b), true) => format!("Turn the table to {} {} and return home", machine.post.b_letter, deg(b)),
                (Some(b), false) => format!("Turn the table to {} {}", machine.post.b_letter, deg(b)),
                _ => "Return home".to_owned(),
            });
            current_cut = None;
        }
        if prog.release_at == Some(i) {
            if settings.wire_off_between_cuts || !settings.between_cuts.trim().is_empty() {
                // Treat it as a cut of its own.
                current_cut = None;
            } else {
                out.comment("Release - cut the part free from the foam under it");
                current_cut = cut;
            }
        }
        if cut != current_cut {
            // Leaving a cut for the moves to the next one.
            if cut.is_none() && current_cut.is_some() {
                if settings.wire_off_between_cuts {
                    out.wire_off(job_name);
                    wire_hot = false;
                }
                let code = settings.between_cuts.clone();
                out.block(&code, job_name);
            }
            current_cut = cut;
            if let Some(c) = cut.and_then(|c| cuts.get(c)) {
                if !wire_hot {
                    out.wire_on(job_name);
                    wire_hot = true;
                }
                if prog.release_at == Some(i) {
                    out.comment("Release - cut the part free from the foam under it");
                } else {
                    out.comment(&format!("{} - {}", c.name, c.kind.label()));
                }
            }
        }
        // A run of feed moves at one angle may be fitted with arcs.
        let feed_move = matches!(m.kind, MoveKind::Cut | MoveKind::Lead);
        if settings.arcs && feed_move {
            let from = prog.from(i);
            let mut j = i;
            while let Some((c2, m2)) = prog.moves.get(j) {
                if *c2 != cut || !matches!(m2.kind, MoveKind::Cut | MoveKind::Lead) {
                    break;
                }
                if (m2.to.b - from.b).abs() > 1.0e-9 || (m2.feed - m.feed).abs() > 1.0e-9 {
                    break;
                }
                j += 1;
            }
            if j > i + 2 {
                let xy = |u: f64, h: f64| {
                    let (x, y) = machine.program_xy(origin, u, h);
                    DVec3::new(x, y, 0.0)
                };
                let pts: Vec<DVec3> =
                    (i..j).filter_map(|k| prog.moves.get(k)).map(|(_, mv)| xy(mv.to.u, mv.to.h)).collect();
                let lim = ArcLimits {
                    tolerance: if units == DisplayUnits::Inches { 0.0127 } else { 0.01 },
                    ..ArcLimits::default()
                };
                let start = xy(from.u, from.h);
                let mut at = start;
                out.set_feed(m.feed);
                for seg in arcfit::fit(start, &pts, &lim) {
                    match seg {
                        Segment::Line(p) => {
                            out.linear(1, &num(p.x), &num(p.y), None);
                            at = p;
                        }
                        Segment::Arc { to, center, cw } => {
                            let (xs, ys) = (num(to.x), num(to.y));
                            let (ii, jj) = (num(center.x - at.x), num(center.y - at.y));
                            let g = if cw { 2 } else { 3 };
                            let f = out.take_feed();
                            out.line(&format!("G{g} X{xs} Y{ys} I{ii} J{jj}{f}"));
                            out.motion = Some(g);
                            out.x = Some(xs);
                            out.y = Some(ys);
                            at = to;
                        }
                    }
                }
                i = j;
                continue;
            }
        }
        let (x, y) = machine.program_xy(origin, m.to.u, m.to.h);
        let b = machine.b_axis.then(|| deg(machine.machine_b(m.to.b)));
        match m.kind {
            MoveKind::Rapid | MoveKind::Rotate => out.linear(0, &num(x), &num(y), b),
            MoveKind::Cut | MoveKind::Lead => {
                out.set_feed(m.feed);
                out.linear(1, &num(x), &num(y), b);
            }
        }
        i += 1;
    }
    if wire_hot {
        out.wire_off(job_name);
    }
    out.block(&settings.footer, job_name);
    if settings.percent {
        out.line("%");
    }

    // Travel limits.
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    for p in std::iter::once(prog.start).chain(prog.moves.iter().map(|(_, m)| m.to)) {
        let (x, y) = machine.machine_xy(p.u, p.h);
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    let eps = 1.0e-6;
    let fmt = |mm: f64| units.fmt_len(mm);
    if lo.0 < -eps || hi.0 > machine.travel_x + eps {
        warnings.push(format!(
            "X runs from {} to {}, past the machine's travel of 0 to {}",
            fmt(lo.0),
            fmt(hi.0),
            fmt(machine.travel_x)
        ));
    }
    if lo.1 < -eps || hi.1 > machine.travel_y + eps {
        warnings.push(format!(
            "Y runs from {} to {}, past the machine's travel of 0 to {}",
            fmt(lo.1),
            fmt(hi.1),
            fmt(machine.travel_y)
        ));
    }
    let lines = out.text.lines().count();
    let mut text = out.text;
    // Warnings go in the program too, where the operator will see them.
    if !warnings.is_empty() {
        let mut head = String::new();
        for w in &warnings {
            let _ = writeln!(head, "(WARNING: {})", ascii(w).replace('[', " "));
        }
        text = if settings.percent { text.replacen("%\n", &format!("%\n{head}"), 1) } else { format!("{head}{text}") };
    }
    Posted { text, lines: lines + warnings.len(), seconds: prog.duration() + settings.preheat, warnings }
}

fn trim(s: String) -> String {
    let t = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    if t == "-0" { "0".into() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_bad_letters_are_refused() {
        let mut s = PostSettings::default();
        let p = json!({ "wire_on": "M64 P0", "decimals": 2, "b_letter": "a", "line_numbers": true });
        assert!(s.apply(&p).is_ok());
        assert_eq!(s.b_letter, "A");
        let mut back = PostSettings::default();
        assert!(back.apply(&s.to_json()).is_ok());
        assert_eq!(back, s);
        assert!(s.apply(&json!({ "b_letter": "X" })).is_err());
        assert!(s.apply(&json!({ "extension": ".n c" })).is_err());
        assert!(s.apply(&json!({ "decimals": null })).is_ok());
        assert_eq!(s.decimals, None);
    }
}
