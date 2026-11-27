mod verify;

use std::collections::HashSet;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

pub use verify::{ChoiceLabelIssue, UnreachableCommand, VerifyReport};

/// An ordered set of scenes with unique IDs and resolved jump destinations.
#[derive(Debug)]
pub struct Script {
    scenes: Vec<Scene>,
}

impl Script {
    /// Parses a single extended PP YAML document and validates its structure.
    pub fn from_yaml(yaml: &str) -> Result<Self, LoadError> {
        let scenes: Vec<Scene> = serde_saphyr::from_str(yaml)?;
        let mut ids = HashSet::new();
        for scene in &scenes {
            if scene.scene.trim().is_empty() {
                return Err(LoadError::Invalid("scene ID is empty".into()));
            }
            if !ids.insert(scene.scene.as_str()) {
                return Err(LoadError::Invalid(format!(
                    "duplicate scene ID {:?}",
                    scene.scene
                )));
            }
        }
        for scene in &scenes {
            if scene.prob.is_some_and(|prob| prob > 100) {
                return Err(LoadError::Invalid(format!(
                    "scene {:?}: prob must be between 0 and 100",
                    scene.scene
                )));
            }
            if let Some(condition) = &scene.condition {
                if condition.iter().next().is_none() {
                    return Err(LoadError::Invalid(format!(
                        "scene {:?}: if must list at least one condition",
                        scene.scene
                    )));
                }
                for condition in condition.iter() {
                    condition.validate().map_err(|message| {
                        LoadError::Invalid(format!("scene {:?}: {message}", scene.scene))
                    })?;
                }
            }
            validate_commands(
                &scene.commands,
                &ids,
                &format!("scene {:?} commands", scene.scene),
            )?;
        }
        Ok(Self { scenes })
    }

    pub fn scenes(&self) -> &[Scene] {
        &self.scenes
    }

    pub fn scene(&self, id: &str) -> Option<&Scene> {
        self.scenes.iter().find(|scene| scene.scene == id)
    }
}

#[derive(Debug)]
pub enum LoadError {
    Yaml(Box<serde_saphyr::Error>),
    Invalid(String),
}

impl From<serde_saphyr::Error> for LoadError {
    fn from(error: serde_saphyr::Error) -> Self {
        Self::Yaml(Box::new(error))
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Yaml(error) => error.fmt(f),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Yaml(error) => Some(error.as_ref()),
            Self::Invalid(_) => None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub scene: String,
    pub when: Option<NonZeroU32>,
    pub phase: Option<usize>,
    #[serde(rename = "if")]
    pub condition: Option<SceneCondition>,
    pub prob: Option<u8>,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Text {
        speaker: Option<String>,
        text: Vec<String>,
    },
    Left {
        #[serde(deserialize_with = "Option::deserialize")]
        asset: Option<String>,
    },
    Right {
        #[serde(deserialize_with = "Option::deserialize")]
        asset: Option<String>,
    },
    Background {
        asset: String,
    },
    Wait {
        ms: u64,
    },
    Effect {
        effect: String,
    },
    Se {
        asset: String,
    },
    Bgm(Bgm),
    Choice {
        options: Vec<ChoiceOption>,
    },
    If {
        condition: Condition,
        then: Vec<Command>,
        #[serde(rename = "else", default)]
        otherwise: Vec<Command>,
    },
    Switch {
        stat: Option<String>,
        flag: Option<String>,
        cases: Vec<CaseEntry>,
        #[serde(default)]
        default: Vec<Command>,
    },
    Go {
        scene: String,
    },
    End {},
    Set {
        stat: Option<String>,
        flag: Option<String>,
        value: Value,
    },
    Add {
        stat: String,
        value: u64,
    },
    Sub {
        stat: String,
        value: u64,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Bgm {
    Start { asset: String },
    Stop {},
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub text: String,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseEntry {
    pub case: Case,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub value: Value,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Value {
    Integer(i64),
    Boolean(bool),
}

/// A scene entry condition: one comparison, or a list that must all match.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SceneCondition {
    One(Condition),
    All(Vec<Condition>),
}

impl SceneCondition {
    pub fn iter(&self) -> std::slice::Iter<'_, Condition> {
        match self {
            Self::One(condition) => std::slice::from_ref(condition).iter(),
            Self::All(conditions) => conditions.iter(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub stat: Option<String>,
    pub flag: Option<String>,
    pub eq: Option<Value>,
    pub ne: Option<Value>,
    pub lt: Option<Value>,
    pub le: Option<Value>,
    pub gt: Option<Value>,
    pub ge: Option<Value>,
}

impl Condition {
    fn validate(&self) -> Result<(), &'static str> {
        let is_flag = state_kind(&self.stat, &self.flag)?;
        let comparisons = [self.eq, self.ne, self.lt, self.le, self.gt, self.ge];
        if comparisons.iter().flatten().count() != 1 {
            return Err("condition requires exactly one comparison");
        }
        if is_flag && self.eq.is_none() && self.ne.is_none() {
            return Err("flag comparisons must use eq or ne");
        }
        for value in comparisons.into_iter().flatten() {
            validate_value(is_flag, value)?;
        }
        Ok(())
    }
}

fn state_kind(stat: &Option<String>, flag: &Option<String>) -> Result<bool, &'static str> {
    match (stat, flag) {
        (Some(_), None) => Ok(false),
        (None, Some(_)) => Ok(true),
        _ => Err("expected exactly one of stat or flag"),
    }
}

fn validate_value(is_flag: bool, value: Value) -> Result<(), &'static str> {
    match (is_flag, value) {
        (true, Value::Boolean(_)) | (false, Value::Integer(_)) => Ok(()),
        (true, _) => Err("flag value must be a boolean"),
        (false, _) => Err("stat value must be an integer"),
    }
}

impl Command {
    /// Each returned list is an alternative branch. Empty fallbacks fall through.
    pub(crate) fn branches(&self) -> Vec<(String, &[Command])> {
        match self {
            Self::Choice { options } => options
                .iter()
                .enumerate()
                .map(|(i, option)| (format!("options[{i}].commands"), option.commands.as_slice()))
                .collect(),
            Self::If {
                then, otherwise, ..
            } => {
                vec![("then".into(), then), ("else".into(), otherwise)]
            }
            Self::Switch { cases, default, .. } => cases
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    (
                        format!("cases[{i}].case.commands"),
                        entry.case.commands.as_slice(),
                    )
                })
                .chain(std::iter::once(("default".into(), default.as_slice())))
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn validate_commands(
    commands: &[Command],
    ids: &HashSet<&str>,
    path: &str,
) -> Result<(), LoadError> {
    for (i, command) in commands.iter().enumerate() {
        let path = format!("{path}[{i}]");
        let invalid = |message: &str| LoadError::Invalid(format!("{path}: {message}"));
        match command {
            Command::Text { text, .. } if text.is_empty() => {
                return Err(invalid("text requires at least one line"));
            }
            Command::Choice { options } if options.is_empty() => {
                return Err(invalid("choice requires at least one option"));
            }
            Command::If { condition, .. } => condition.validate().map_err(invalid)?,
            Command::Switch {
                stat, flag, cases, ..
            } => {
                let is_flag = state_kind(stat, flag).map_err(invalid)?;
                for entry in cases {
                    validate_value(is_flag, entry.case.value).map_err(invalid)?;
                }
            }
            Command::Set { stat, flag, value } => {
                validate_value(state_kind(stat, flag).map_err(invalid)?, *value)
                    .map_err(invalid)?;
            }
            Command::Go { scene } if !ids.contains(scene.as_str()) => {
                return Err(invalid(&format!("unknown scene ID {scene:?}")));
            }
            _ => {}
        }
        for (branch, commands) in command.branches() {
            validate_commands(commands, ids, &format!("{path}.{branch}"))?;
        }
    }
    Ok(())
}
