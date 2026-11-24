//! Values the session reports to its host: pauses, cues, branch traces and the view.

use std::collections::BTreeMap;

use serde::Serialize;

use super::scenario;
use crate::script::{Condition, Value};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Pause {
    Ended,
    Text {
        speaker: Option<String>,
        text: String,
    },
    Choice {
        options: Vec<String>,
    },
    Wait {
        ms: u64,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StartBgm { asset: String },
    StopBgm,
    PlaySe { asset: String },
    PlayEffect { effect: String },
}

/// Control-flow decisions made during the latest successful input.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Branch {
    If {
        at: Location,
        condition: Condition,
        matched: bool,
    },
    Switch {
        at: Location,
        variable: String,
        value: Value,
        cases: Vec<Value>,
        selected: Option<usize>,
    },
    Go {
        at: Location,
        scene: String,
    },
}

/// Identifies the same command in playback and in the static script tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    pub scene: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct View<'a> {
    pub scene: Option<&'a str>,
    pub pause: &'a Pause,
    pub location: Option<&'a Location>,
    pub background: Option<&'a str>,
    pub left: Option<&'a str>,
    pub right: Option<&'a str>,
    pub bgm: Option<&'a str>,
    #[serde(serialize_with = "scenario::serialize_stats")]
    pub stats: &'a BTreeMap<String, i64>,
    pub flags: &'a BTreeMap<String, bool>,
}
