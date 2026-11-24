use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    script::Script,
    session::{Branch, Event, Scenario, Session, StateChange, View},
};

/// Read the complete command tree without starting a playback session.
#[wasm_bindgen]
pub fn script_flow(source: &str) -> Result<String, JsValue> {
    let script = Script::from_yaml(source).map_err(js_error)?;
    serde_json::to_string(script.scenes()).map_err(js_error)
}

#[wasm_bindgen]
pub struct WebSession(Session);

#[derive(Serialize)]
struct Update<'a> {
    state: View<'a>,
    events: Vec<Event>,
    branches: &'a [Branch],
    changes: &'a [StateChange],
}

#[wasm_bindgen]
impl WebSession {
    #[wasm_bindgen(constructor)]
    pub fn new(script: &str, scenario: &str, player_name: &str) -> Result<WebSession, JsValue> {
        let script = Script::from_yaml(script).map_err(js_error)?;
        let scenario = Scenario::from_yaml(scenario).map_err(js_error)?;
        Session::new(script, scenario, player_name)
            .map(Self)
            .map_err(js_error)
    }

    pub fn start(&mut self) -> Result<String, JsValue> {
        let events = self.0.start_configured().map_err(js_error)?;
        self.update(events)
    }

    pub fn trigger(&mut self, turn: u32, phase: usize) -> Result<String, JsValue> {
        let events = self
            .0
            .trigger(turn, phase, || (js_sys::Math::random() * 100.0) as u8)
            .map_err(js_error)?;
        self.update(events)
    }

    /// Same as `trigger`, with reproducible rolls derived from `seed`.
    pub fn trigger_seeded(
        &mut self,
        turn: u32,
        phase: usize,
        seed: u32,
    ) -> Result<String, JsValue> {
        let mut rolls = SeededRolls::new(seed);
        let events = self
            .0
            .trigger(turn, phase, || rolls.next())
            .map_err(js_error)?;
        self.update(events)
    }

    pub fn set_stat(&mut self, id: &str, value: f64) -> Result<(), JsValue> {
        if value.fract() != 0.0
            || !(-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&value)
        {
            return Err(js_error(format!("stat {id:?} must be a safe integer")));
        }
        self.0.set_stat(id, value as i64).map_err(js_error)
    }

    pub fn set_flag(&mut self, id: &str, value: bool) -> Result<(), JsValue> {
        self.0.set_flag(id, value).map_err(js_error)
    }

    /// The current state without running any command.
    pub fn view(&self) -> Result<String, JsValue> {
        self.update(Vec::new())
    }

    pub fn advance(&mut self) -> Result<String, JsValue> {
        let events = self.0.advance().map_err(js_error)?;
        self.update(events)
    }

    pub fn choose(&mut self, index: usize) -> Result<String, JsValue> {
        let events = self.0.choose(index).map_err(js_error)?;
        self.update(events)
    }

    pub fn resume(&mut self) -> Result<String, JsValue> {
        let events = self.0.resume().map_err(js_error)?;
        self.update(events)
    }
}

impl WebSession {
    fn update(&self, events: Vec<Event>) -> Result<String, JsValue> {
        serde_json::to_string(&Update {
            state: self.0.view(),
            events,
            branches: self.0.branches(),
            changes: self.0.state_changes(),
        })
        .map_err(js_error)
    }
}

/// xorshift32, mapped to percentage rolls in 0..100.
struct SeededRolls(u32);

impl SeededRolls {
    fn new(seed: u32) -> Self {
        Self(seed ^ 0x9e37_79b9 | 1)
    }

    fn next(&mut self) -> u8 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x % 100) as u8
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}
