//! Interpreter that runs until dialogue, a choice, a timer, or the end of a scene.

mod scenario;

use std::{collections::BTreeMap, vec::IntoIter};

use serde::Serialize;

use crate::{
    presentation::{Cue, Presentation, Side},
    script::{ChoiceOption, Command, Condition, Script, Value},
};
use scenario::State;

pub use scenario::{Scenario, Stat, StateChange, StateKind};

const COMMANDS_PER_INPUT: usize = 10_000;

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

struct Frame {
    path: String,
    commands: std::iter::Enumerate<IntoIter<Command>>,
}

impl Frame {
    fn new(path: String, commands: Vec<Command>) -> Self {
        Self {
            path,
            commands: commands.into_iter().enumerate(),
        }
    }
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

pub struct Session {
    script: Script,
    scenario: Scenario,
    player_name: String,
    state: State,
    presentation: Presentation,
    scene: Option<String>,
    frames: Vec<Frame>,
    location: Option<Location>,
    pause: Pause,
    speaker: Option<String>,
    lines: IntoIter<String>,
    choices: Vec<ChoiceOption>,
    branches: Vec<Branch>,
}

impl Session {
    pub fn new(script: Script, scenario: Scenario, player_name: &str) -> Result<Self, String> {
        scenario
            .validate(&script)
            .map_err(|errors| errors.join("\n"))?;
        let state = State::new(&scenario);
        Ok(Self {
            script,
            scenario,
            player_name: player_name.into(),
            state,
            presentation: Presentation::default(),
            scene: None,
            frames: Vec::new(),
            location: None,
            pause: Pause::Ended,
            speaker: None,
            lines: Vec::new().into_iter(),
            choices: Vec::new(),
            branches: Vec::new(),
        })
    }

    pub fn view(&self) -> View<'_> {
        View {
            scene: self.scene.as_deref(),
            pause: &self.pause,
            location: self.location.as_ref(),
            background: self.presentation.background(),
            left: self.presentation.portrait(Side::Left),
            right: self.presentation.portrait(Side::Right),
            bgm: self.presentation.bgm(),
            stats: &self.state.stats,
            flags: &self.state.flags,
        }
    }

    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Values changed while processing the latest input, in command order.
    pub fn state_changes(&self) -> &[StateChange] {
        &self.state.changes
    }

    fn clear_trace(&mut self) {
        self.branches.clear();
        self.state.changes.clear();
    }

    /// Start at the entry declared by the scenario configuration.
    pub fn start_configured(&mut self) -> Result<Vec<Event>, String> {
        self.start(&self.scenario.start_scene.clone())
    }

    /// Start a scene directly, using the same entry semantics as a script jump.
    pub fn start(&mut self, id: &str) -> Result<Vec<Event>, String> {
        if self.pause != Pause::Ended {
            return Err("the current scene is still active".into());
        }
        self.enter(id)?;
        self.run()
    }

    /// Run the first scheduled event that matches. The caller supplies a fresh
    /// percentage roll in 0..100 for each candidate that needs a roll.
    pub fn trigger(
        &mut self,
        turn: u32,
        phase: usize,
        mut roll: impl FnMut() -> u8,
    ) -> Result<Vec<Event>, String> {
        if self.pause != Pause::Ended {
            return Err("the current scene is still active".into());
        }
        if !(1..=self.scenario.max_turns.get()).contains(&turn) {
            return Err(format!(
                "turn {turn} is outside 1..={}",
                self.scenario.max_turns
            ));
        }
        if phase >= self.scenario.phases.len() {
            return Err(format!("phase ID {phase} is out of range"));
        }
        for scene in self.script.scenes() {
            let scheduled = scene.when.is_some()
                || scene.phase.is_some()
                || scene.condition.is_some()
                || scene.prob.is_some();
            if !scheduled
                || scene.when.is_some_and(|value| value.get() != turn)
                || scene.phase.is_some_and(|value| value != phase)
            {
                continue;
            }
            if let Some(condition) = &scene.condition
                && !self.state.matches(condition)?
            {
                continue;
            }
            match scene.prob.unwrap_or(100) {
                0 => continue,
                100 => {}
                probability => {
                    let value = roll();
                    if value >= 100 {
                        return Err("probability roll must be in 0..100".into());
                    }
                    if value >= probability {
                        continue;
                    }
                }
            }
            return self.start(&scene.scene.clone());
        }
        self.clear_trace();
        Ok(Vec::new())
    }

    pub fn advance(&mut self) -> Result<Vec<Event>, String> {
        if !matches!(self.pause, Pause::Text { .. }) {
            return Err("no dialogue to advance".into());
        }
        self.clear_trace();
        if self.show_line() {
            return Ok(Vec::new());
        }
        self.run()
    }

    pub fn choose(&mut self, index: usize) -> Result<Vec<Event>, String> {
        if !matches!(self.pause, Pause::Choice { .. }) {
            return Err("no choice is active".into());
        }
        if index >= self.choices.len() {
            return Err(format!("choice index {index} is out of range"));
        }
        let commands = std::mem::take(&mut self.choices)
            .swap_remove(index)
            .commands;
        let path = &self
            .location
            .as_ref()
            .expect("active choice has a location")
            .path;
        self.frames
            .push(Frame::new(format!("{path}.options.{index}"), commands));
        self.run()
    }

    /// The host calls this when the currently requested delay has elapsed.
    pub fn resume(&mut self) -> Result<Vec<Event>, String> {
        if !matches!(self.pause, Pause::Wait { .. }) {
            return Err("no timer is active".into());
        }
        self.run()
    }

    fn enter(&mut self, id: &str) -> Result<(), String> {
        let scene = self
            .script
            .scene(id)
            .ok_or_else(|| format!("unknown scene {id:?}"))?;
        self.scene = Some(id.into());
        self.frames.clear();
        self.location = None;
        self.frames
            .push(Frame::new("commands".into(), scene.commands.clone()));
        Ok(())
    }

    fn finish(&mut self) {
        self.frames.clear();
        self.pause = Pause::Ended;
    }

    fn show_line(&mut self) -> bool {
        if let Some(line) = self.lines.next() {
            self.pause = Pause::Text {
                speaker: self.speaker.clone(),
                text: line.replace("{NAME}", &self.player_name),
            };
            return true;
        }
        false
    }

    fn run(&mut self) -> Result<Vec<Event>, String> {
        self.clear_trace();
        let result = self.run_until_pause();
        if result.is_err() {
            self.clear_trace();
            self.finish();
        }
        result
    }

    fn run_until_pause(&mut self) -> Result<Vec<Event>, String> {
        let mut events = Vec::new();
        for _ in 0..COMMANDS_PER_INPUT {
            while self
                .frames
                .last()
                .is_some_and(|frame| frame.commands.len() == 0)
            {
                self.frames.pop();
            }
            let Some(frame) = self.frames.last_mut() else {
                self.location = None;
                self.finish();
                return Ok(events);
            };
            let (index, command) = frame.commands.next().expect("nonempty command frame");
            let at = Location {
                scene: self.scene.clone().expect("command frame has a scene"),
                path: format!("{}.{}", frame.path, index),
            };
            self.location = Some(at.clone());
            if let Some(cue) = self.presentation.apply(&command) {
                match cue {
                    Cue::StartBgm { asset } => events.push(Event::StartBgm {
                        asset: asset.into(),
                    }),
                    Cue::StopBgm => events.push(Event::StopBgm),
                    Cue::PlaySe { asset } => events.push(Event::PlaySe {
                        asset: asset.into(),
                    }),
                    Cue::PlayEffect { effect } => events.push(Event::PlayEffect {
                        effect: effect.into(),
                    }),
                    Cue::Wait { ms } if ms != 0 => {
                        self.pause = Pause::Wait { ms };
                        return Ok(events);
                    }
                    _ => {}
                }
                continue;
            }
            match command {
                Command::Text { speaker, text } => {
                    self.speaker = match speaker.as_deref() {
                        Some("protagonist") => Some(self.player_name.clone()),
                        Some(id) => Some(
                            self.scenario
                                .characters
                                .get(id)
                                .cloned()
                                .ok_or_else(|| format!("unknown speaker {id:?}"))?,
                        ),
                        None => None,
                    };
                    self.lines = text.into_iter();
                    self.show_line();
                    return Ok(events);
                }
                Command::Choice { options } => {
                    self.pause = Pause::Choice {
                        options: options.iter().map(|option| option.text.clone()).collect(),
                    };
                    self.choices = options;
                    return Ok(events);
                }
                Command::If {
                    condition,
                    then,
                    otherwise,
                } => {
                    let matched = self.state.matches(&condition)?;
                    let branch = if matched { "then" } else { "else" };
                    let path = format!("{}.{branch}", at.path);
                    self.branches.push(Branch::If {
                        at,
                        condition,
                        matched,
                    });
                    let commands = if matched { then } else { otherwise };
                    self.frames.push(Frame::new(path, commands));
                }
                Command::Switch {
                    stat,
                    flag,
                    cases,
                    default,
                } => {
                    let value = self.state.get(stat.as_deref(), flag.as_deref())?;
                    let selected = cases.iter().position(|entry| entry.case.value == value);
                    let branch =
                        selected.map_or_else(|| "default".into(), |index| format!("cases.{index}"));
                    let path = format!("{}.{branch}", at.path);
                    self.branches.push(Branch::Switch {
                        at,
                        variable: stat.or(flag).expect("validated state reference"),
                        value,
                        cases: cases.iter().map(|entry| entry.case.value).collect(),
                        selected,
                    });
                    let commands = selected
                        .and_then(|index| cases.into_iter().nth(index))
                        .map(|entry| entry.case.commands)
                        .unwrap_or(default);
                    self.frames.push(Frame::new(path, commands));
                }
                Command::Go { scene } => {
                    self.enter(&scene)?;
                    self.branches.push(Branch::Go { at, scene });
                }
                Command::End {} => {
                    self.finish();
                    return Ok(events);
                }
                Command::Set { stat, flag, value } => match value {
                    Value::Integer(value) => self.state.update_stat(
                        &self.scenario,
                        &stat.expect("validated stat"),
                        |_| value.into(),
                    )?,
                    Value::Boolean(value) => {
                        self.state.set_flag(&flag.expect("validated flag"), value)?
                    }
                },
                Command::Add { stat, value } => {
                    self.state
                        .update_stat(&self.scenario, &stat, |current| current + i128::from(value))?
                }
                Command::Sub { stat, value } => {
                    self.state
                        .update_stat(&self.scenario, &stat, |current| current - i128::from(value))?
                }
                _ => unreachable!("presentation command was handled"),
            }
        }
        Err("script exceeded the command limit without waiting for input".into())
    }
}
