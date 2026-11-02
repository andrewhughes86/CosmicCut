//! Operations: the job's cuts in groups, each starting with a machine stop if it needs one: the
//! operator turns the stock by hand, changes the wire, reads a message.

use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::params;

/// What the machine does at the start of an operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stop {
    /// Carry straight on.
    #[default]
    None,
    /// `M0`: stop until the operator resumes.
    Pause,
    /// `M1`: stop only if the controller's optional stop is on.
    Optional,
}

impl Stop {
    pub const IDS: &'static [&'static str] = &["none", "m0", "m1"];

    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Pause => "m0",
            Self::Optional => "m1",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "none" => Some(Self::None),
            "m0" => Some(Self::Pause),
            "m1" => Some(Self::Optional),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No stop",
            Self::Pause => "Stop (M0)",
            Self::Optional => "Optional stop (M1)",
        }
    }

    pub fn code(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Pause => Some("M0"),
            Self::Optional => Some("M1"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Operation {
    pub id: String,
    /// What it is for, after "Op N" ("Turn the stock a quarter").
    pub name: String,
    pub stop: Stop,
    /// Shown to the operator on the controller when the operation starts (`(MSG, …)`).
    pub message: String,
    /// For the operator: written into the program as comments.
    pub notes: String,
}

/// The id of the operation every job starts with.
pub const FIRST: &str = "op.0";

impl Operation {
    pub fn first() -> Self {
        Self { id: FIRST.into(), ..Self::default() }
    }

    /// "Op 2" or "Op 2 · Turn the stock a quarter", for its place `n` (from 0).
    pub fn title(&self, n: usize) -> String {
        if self.name.trim().is_empty() {
            format!("Op {}", n + 1)
        } else {
            format!("Op {} · {}", n + 1, self.name.trim())
        }
    }

    /// Whether the machine waits for the operator when it starts.
    pub fn pauses(&self) -> bool {
        self.stop != Stop::None || !self.message.trim().is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "stop": self.stop.id(),
            "message": self.message,
            "notes": self.notes,
        })
    }

    /// Applies the fields present in `p`.
    pub fn apply(&mut self, p: &Value) -> EngineResult<()> {
        let text = |k: &str| -> EngineResult<Option<String>> {
            Ok(params::opt_str(p, k)?.map(|t| t.replace("\r\n", "\n").chars().take(2000).collect()))
        };
        if let Some(n) = text("name")? {
            self.name = n.trim().to_owned();
        }
        if let Some(s) = params::opt_str(p, "stop")? {
            self.stop = Stop::from_id(s)
                .ok_or_else(|| EngineError::InvalidParams(format!("`stop` must be one of {:?}", Stop::IDS)))?;
        }
        if let Some(m) = text("message")? {
            // A message is one line: the controller shows it in a box.
            self.message = m.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        }
        if let Some(n) = text("notes")? {
            self.notes = n;
        }
        Ok(())
    }

    pub fn from_json(v: &Value) -> EngineResult<Self> {
        let mut op = Self { id: v.get("id").and_then(Value::as_str).unwrap_or(FIRST).to_owned(), ..Self::default() };
        let mut p = v.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("id");
        }
        op.apply(&p)?;
        Ok(op)
    }

    pub fn properties(&self) -> Vec<(String, String)> {
        let mut out = vec![("Machine stop".into(), self.stop.label().into())];
        if !self.message.is_empty() {
            out.push(("Message".into(), self.message.clone()));
        }
        if !self.notes.is_empty() {
            out.push(("Notes".into(), self.notes.clone()));
        }
        out
    }
}
