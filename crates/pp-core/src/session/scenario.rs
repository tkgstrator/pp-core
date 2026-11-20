use std::{collections::BTreeMap, num::NonZeroU32};

use serde::{Deserialize, Serialize, Serializer};

use crate::script::{Command, Condition, Script, Value, VerifyReport};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub start_scene: String,
    pub max_turns: NonZeroU32,
    /// Display names in phase order. The array index is the phase ID.
    pub phases: Vec<String>,
    #[serde(default)]
    pub characters: BTreeMap<String, String>,
    #[serde(default)]
    pub stats: BTreeMap<String, Stat>,
    #[serde(default)]
    pub flags: BTreeMap<String, bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stat {
    pub initial: i64,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

impl Scenario {
    pub fn from_yaml(source: &str) -> Result<Self, serde_saphyr::Error> {
        serde_saphyr::from_str(source)
    }

    /// Check configuration and script references, then report static reachability.
    pub fn verify(&self, script: &Script) -> Result<VerifyReport, Vec<String>> {
        self.validate(script)?;
        Ok(script.verify_from(Some(&self.start_scene)))
    }

    pub(super) fn validate(&self, script: &Script) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if script.scene(&self.start_scene).is_none() {
            errors.push(format!("start_scene {:?} does not exist", self.start_scene));
        }
        if self.phases.is_empty() || self.phases.iter().any(|phase| phase.trim().is_empty()) {
            errors.push(
                "phases must list at least one phase and every phase must have a display name"
                    .into(),
            );
        }
        for (id, stat) in &self.stats {
            let min = stat.min.unwrap_or(i64::MIN);
            let max = stat.max.unwrap_or(i64::MAX);
            if !(min..=max).contains(&stat.initial) {
                errors.push(format!("invalid initial value or bounds for stat {id:?}"));
            }
        }
        for scene in script.scenes() {
            let path = format!("scene {:?}", scene.scene);
            if let Some(turn) = scene.when
                && turn > self.max_turns
            {
                errors.push(format!(
                    "{path}.when: turn {turn} exceeds max_turns {}",
                    self.max_turns
                ));
            }
            if let Some(phase) = scene.phase
                && phase >= self.phases.len()
            {
                errors.push(format!("{path}.phase: phase ID {phase} is out of range"));
            }
            if let Some(condition) = &scene.condition {
                self.validate_state(
                    condition.stat.as_deref(),
                    condition.flag.as_deref(),
                    &format!("{path}.if"),
                    &mut errors,
                );
            }
            self.validate_commands(&scene.commands, &format!("{path}.commands"), &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_commands(&self, commands: &[Command], prefix: &str, errors: &mut Vec<String>) {
        for (index, command) in commands.iter().enumerate() {
            let path = format!("{prefix}[{index}]");
            match command {
                Command::Text {
                    speaker: Some(id), ..
                } if id != "protagonist" && !self.characters.contains_key(id) => {
                    errors.push(format!("{path}: unknown speaker {id:?}"));
                }
                Command::If { condition, .. } => self.validate_state(
                    condition.stat.as_deref(),
                    condition.flag.as_deref(),
                    &path,
                    errors,
                ),
                Command::Set { stat, flag, .. } | Command::Switch { stat, flag, .. } => {
                    self.validate_state(stat.as_deref(), flag.as_deref(), &path, errors);
                }
                Command::Add { stat, .. } | Command::Sub { stat, .. } => {
                    self.validate_state(Some(stat), None, &path, errors);
                }
                _ => {}
            }
            for (branch, commands) in command.branches() {
                self.validate_commands(commands, &format!("{path}.{branch}"), errors);
            }
        }
    }

    fn validate_state(
        &self,
        stat: Option<&str>,
        flag: Option<&str>,
        path: &str,
        errors: &mut Vec<String>,
    ) {
        if let Some(stat) = stat
            && !self.stats.contains_key(stat)
        {
            errors.push(format!("{path}: unknown stat {stat:?}"));
        }
        if let Some(flag) = flag
            && !self.flags.contains_key(flag)
        {
            errors.push(format!("{path}: unknown flag {flag:?}"));
        }
    }
}

/// A value changed by a script command, after applying the configured bounds.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct StateChange {
    pub kind: StateKind,
    pub id: String,
    #[serde(serialize_with = "serialize_state_value")]
    pub before: Value,
    #[serde(serialize_with = "serialize_state_value")]
    pub after: Value,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateKind {
    Stat,
    Flag,
}

// Decimal strings preserve the full i64 range in browser state inspectors.
fn serialize_state_value<S: Serializer>(value: &Value, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Value::Integer(value) => serializer.collect_str(value),
        Value::Boolean(value) => serializer.serialize_bool(*value),
    }
}

pub(super) fn serialize_stats<S: Serializer>(
    stats: &BTreeMap<String, i64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(stats.iter().map(|(id, value)| (id, value.to_string())))
}

pub(super) struct State {
    pub changes: Vec<StateChange>,
    pub stats: BTreeMap<String, i64>,
    pub flags: BTreeMap<String, bool>,
}

impl State {
    pub fn new(scenario: &Scenario) -> Self {
        Self {
            stats: scenario
                .stats
                .iter()
                .map(|(id, stat)| (id.clone(), stat.initial))
                .collect(),
            flags: scenario.flags.clone(),
            changes: Vec::new(),
        }
    }

    pub fn get(&self, stat: Option<&str>, flag: Option<&str>) -> Result<Value, String> {
        if let Some(id) = stat {
            return self
                .stats
                .get(id)
                .copied()
                .map(Value::Integer)
                .ok_or_else(|| format!("unknown stat {id:?}"));
        }
        let id = flag.expect("validated state reference");
        self.flags
            .get(id)
            .copied()
            .map(Value::Boolean)
            .ok_or_else(|| format!("unknown flag {id:?}"))
    }

    pub fn update_stat(
        &mut self,
        scenario: &Scenario,
        id: &str,
        update: impl FnOnce(i128) -> i128,
    ) -> Result<(), String> {
        let value = self
            .stats
            .get_mut(id)
            .ok_or_else(|| format!("unknown stat {id:?}"))?;
        let definition = &scenario.stats[id];
        let min = definition.min.unwrap_or(i64::MIN);
        let max = definition.max.unwrap_or(i64::MAX);
        let before = *value;
        let after = update(i128::from(before)).clamp(i128::from(min), i128::from(max)) as i64;
        *value = after;
        self.record(
            StateKind::Stat,
            id,
            Value::Integer(before),
            Value::Integer(after),
        );
        Ok(())
    }

    pub fn set_flag(&mut self, id: &str, value: bool) -> Result<(), String> {
        let flag = self
            .flags
            .get_mut(id)
            .ok_or_else(|| format!("unknown flag {id:?}"))?;
        let before = std::mem::replace(flag, value);
        self.record(
            StateKind::Flag,
            id,
            Value::Boolean(before),
            Value::Boolean(value),
        );
        Ok(())
    }

    fn record(&mut self, kind: StateKind, id: &str, before: Value, after: Value) {
        if before != after {
            self.changes.push(StateChange {
                kind,
                id: id.into(),
                before,
                after,
            });
        }
    }

    pub fn matches(&self, condition: &Condition) -> Result<bool, String> {
        let value = self.get(condition.stat.as_deref(), condition.flag.as_deref())?;
        if let Some(eq) = condition.eq {
            return Ok(value == eq);
        }
        if let Some(ne) = condition.ne {
            return Ok(value != ne);
        }
        let Value::Integer(value) = value else {
            unreachable!("validated comparison")
        };
        Ok(
            match (condition.lt, condition.le, condition.gt, condition.ge) {
                (Some(Value::Integer(rhs)), _, _, _) => value < rhs,
                (_, Some(Value::Integer(rhs)), _, _) => value <= rhs,
                (_, _, Some(Value::Integer(rhs)), _) => value > rhs,
                (_, _, _, Some(Value::Integer(rhs))) => value >= rhs,
                _ => unreachable!("validated comparison"),
            },
        )
    }
}
