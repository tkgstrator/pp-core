use std::collections::{HashMap, HashSet, VecDeque};

use super::{Command, Script};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub unreachable_scenes: Vec<String>,
    pub unreachable_commands: Vec<UnreachableCommand>,
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.unreachable_scenes.is_empty() && self.unreachable_commands.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnreachableCommand {
    pub scene: String,
    /// Zero-based path relative to the scene, such as `commands[3].else[0]`.
    pub path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChoiceLabelIssue {
    pub scene: String,
    /// Zero-based path to the offending option's text.
    pub path: String,
    pub label: String,
    pub width: usize,
    pub multiline: bool,
}

impl Script {
    /// Checks every choice label, including debug-only scenes and nested branches.
    /// The frontend supplies the one-line width limit and its text measurement.
    pub fn verify_choice_labels(
        &self,
        max_width: usize,
        measure: impl Fn(&str) -> usize,
    ) -> Vec<ChoiceLabelIssue> {
        let mut issues = Vec::new();
        for scene in self.scenes() {
            check_choices(
                &scene.commands,
                &scene.scene,
                "commands",
                max_width,
                &measure,
                &mut issues,
            );
        }
        issues
    }

    /// Checks reachability from the first scene and every eligible event entry.
    ///
    /// Conditions and player choices are treated as possible outcomes. This does
    /// not simulate state changes, scheduling priority or probability rolls.
    pub fn verify(&self) -> VerifyReport {
        self.verify_from(self.scenes().first().map(|scene| scene.scene.as_str()))
    }

    pub(crate) fn verify_from(&self, entry: Option<&str>) -> VerifyReport {
        let mut report = VerifyReport::default();
        let indices: HashMap<_, _> = self
            .scenes()
            .iter()
            .enumerate()
            .map(|(i, scene)| (scene.scene.as_str(), i))
            .collect();
        let mut pending: VecDeque<_> = self
            .scenes()
            .iter()
            .enumerate()
            .filter_map(|(i, scene)| {
                let scheduled = scene.when.is_some()
                    || scene.phase.is_some()
                    || scene.condition.is_some()
                    || scene.prob.is_some();
                (entry == Some(scene.scene.as_str()) || (scheduled && scene.prob != Some(0)))
                    .then_some(i)
            })
            .collect();
        let mut visited = HashSet::new();
        while let Some(index) = pending.pop_front() {
            if !visited.insert(index) {
                continue;
            }
            let scene = &self.scenes()[index];
            let mut jumps = Vec::new();
            walk(
                &scene.commands,
                &scene.scene,
                "commands",
                true,
                &mut jumps,
                &mut report.unreachable_commands,
            );
            pending.extend(jumps.into_iter().map(|id| indices[id]));
        }
        report.unreachable_scenes = self
            .scenes()
            .iter()
            .enumerate()
            .filter(|(i, _)| !visited.contains(i))
            .map(|(_, scene)| scene.scene.clone())
            .collect();
        report
    }
}

fn check_choices(
    commands: &[Command],
    scene: &str,
    path: &str,
    max_width: usize,
    measure: &impl Fn(&str) -> usize,
    issues: &mut Vec<ChoiceLabelIssue>,
) {
    for (i, command) in commands.iter().enumerate() {
        let path = format!("{path}[{i}]");
        if let Command::Choice { options } = command {
            for (option_index, option) in options.iter().enumerate() {
                let width = measure(&option.text);
                let multiline = option.text.contains(['\r', '\n']);
                if width > max_width || multiline {
                    issues.push(ChoiceLabelIssue {
                        scene: scene.into(),
                        path: format!("{path}.options[{option_index}].text"),
                        label: option.text.clone(),
                        width,
                        multiline,
                    });
                }
            }
        }
        for (branch, commands) in command.branches() {
            check_choices(
                commands,
                scene,
                &format!("{path}.{branch}"),
                max_width,
                measure,
                issues,
            );
        }
    }
}

fn walk<'a>(
    commands: &'a [Command],
    scene: &str,
    path: &str,
    mut reachable: bool,
    jumps: &mut Vec<&'a str>,
    dead: &mut Vec<UnreachableCommand>,
) -> bool {
    for (i, command) in commands.iter().enumerate() {
        let path = format!("{path}[{i}]");
        if !reachable {
            dead.push(UnreachableCommand {
                scene: scene.into(),
                path,
            });
            continue;
        }
        match command {
            Command::Go { scene } => {
                jumps.push(scene);
                reachable = false;
            }
            Command::End {} => reachable = false,
            _ => {
                let branches = command.branches();
                if !branches.is_empty() {
                    let mut falls_through = false;
                    let mut case_values = HashSet::new();
                    for (index, (branch, commands)) in branches.into_iter().enumerate() {
                        let possible = match command {
                            Command::Switch { flag, cases, .. } => match cases.get(index) {
                                Some(entry) => case_values.insert(entry.case.value),
                                None => flag.is_none() || case_values.len() < 2,
                            },
                            _ => true,
                        };
                        falls_through |= walk(
                            commands,
                            scene,
                            &format!("{path}.{branch}"),
                            possible,
                            jumps,
                            dead,
                        );
                    }
                    reachable = falls_through;
                }
            }
        }
    }
    reachable
}
