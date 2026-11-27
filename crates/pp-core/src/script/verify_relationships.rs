use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::{Command, Script, Value};

#[derive(Debug, PartialEq, Eq, Hash, Clone, Deserialize, Serialize)]
pub struct ScriptSite {
    pub scene: String,
    /// Zero-based path into the compiled script, relative to the scene.
    pub path: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct SelectedBranch {
    pub site: ScriptSite,
    pub label: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct DialogueWarning {
    pub speaker: String,
    /// None means a direct address without a protagonist reply.
    pub stat: Option<String>,
    pub appearance: ScriptSite,
    pub choice: Option<SelectedBranch>,
    pub condition: Option<SelectedBranch>,
}

impl Script {
    /// Finds paths where a tracked speaker has dialogue without a write to their stat.
    /// Speakers that share a stat are aliases. Nonzero adds/subtracts and integer sets count.
    /// `boundary_cue` optionally ends the current event at effects with that prefix, even
    /// when a `go` then continues to another event. This is a structural warning, not a
    /// simulation of flags, random draws, or stat clamping.
    pub fn verify_relationship_changes(
        &self,
        speakers: &BTreeMap<String, String>,
        boundary_cue: Option<&str>,
    ) -> Vec<DialogueWarning> {
        let graph = Graph::build(self, boundary_cue);
        let mut by_stat: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
        for (speaker, stat) in speakers {
            by_stat.entry(stat).or_default().insert(speaker);
        }
        by_stat
            .into_iter()
            .flat_map(|(stat, aliases)| {
                graph.missing(
                    Some(stat),
                    |_, speaker| aliases.contains(speaker),
                    |node| matches!(node.step, Step::Write(written) if written == stat),
                )
            })
            .collect()
    }

    /// Finds paths with an address to the protagonist and no spoken reply on that path.
    /// The authoring tool identifies direct addresses and audible replies before compilation
    /// erases annotations and merges dialogue, so an aside does not count as a reply.
    pub fn verify_protagonist_replies(
        &self,
        addressed: &[ScriptSite],
        replies: &[ScriptSite],
        boundary_cue: Option<&str>,
    ) -> Vec<DialogueWarning> {
        let addressed: HashSet<_> = addressed.iter().collect();
        let replies: HashSet<_> = replies.iter().collect();
        Graph::build(self, boundary_cue).missing(
            None,
            |node, _| addressed.contains(&node.site),
            |node| replies.contains(&node.site),
        )
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Default)]
struct Trace {
    appearance: Option<usize>,
    choice: Option<usize>,
    condition: Option<usize>,
    covered: bool,
}

enum Step<'a> {
    Branch(Vec<usize>),
    Decision { label: String, choice: bool },
    Text(&'a str),
    Write(&'a str),
    Go(&'a str),
    Boundary,
    End,
    Continue,
}

struct Node<'a> {
    site: ScriptSite,
    step: Step<'a>,
    next: usize,
}

#[derive(Default)]
struct Graph<'a> {
    nodes: Vec<Node<'a>>,
    entries: HashMap<&'a str, usize>,
    starts: Vec<usize>,
}

impl<'a> Graph<'a> {
    fn build(script: &'a Script, boundary: Option<&str>) -> Self {
        let mut graph = Self::default();
        for scene in script.scenes() {
            let end = graph.push(&scene.scene, "commands".into(), Step::End, 0);
            let first = graph.sequence(&scene.commands, &scene.scene, "commands", end, boundary);
            graph.entries.insert(&scene.scene, first);
        }
        let linked: HashSet<_> = graph
            .nodes
            .iter()
            .filter_map(|node| match node.step {
                Step::Go(target) => Some(target),
                _ => None,
            })
            .collect();
        graph.starts = script
            .scenes()
            .iter()
            .enumerate()
            .filter(|(index, scene)| {
                // Independent and debug entries are checked too; a pure jump target inherits its path.
                *index == 0
                    || !linked.contains(scene.scene.as_str())
                    || scene.when.is_some()
                    || scene.phase.is_some()
                    || scene.condition.is_some()
                    || scene.prob.is_some()
            })
            .map(|(_, scene)| graph.entries[scene.scene.as_str()])
            .collect();
        graph
    }

    fn push(&mut self, scene: &str, path: String, step: Step<'a>, next: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            site: ScriptSite {
                scene: scene.into(),
                path,
            },
            step,
            next,
        });
        id
    }

    fn sequence(
        &mut self,
        commands: &'a [Command],
        scene: &str,
        parent: &str,
        mut next: usize,
        boundary: Option<&str>,
    ) -> usize {
        for (index, command) in commands.iter().enumerate().rev() {
            let path = format!("{parent}[{index}]");
            let branches = command.branches();
            let step = if branches.is_empty() {
                match command {
                    Command::Text {
                        speaker: Some(speaker),
                        ..
                    } => Step::Text(speaker),
                    Command::Add { stat, value } | Command::Sub { stat, value } if *value != 0 => {
                        Step::Write(stat)
                    }
                    Command::Set {
                        stat: Some(stat),
                        value: Value::Integer(_),
                        ..
                    } => Step::Write(stat),
                    Command::Go { scene } => Step::Go(scene),
                    Command::End {} => Step::End,
                    Command::Effect { effect }
                        if boundary.is_some_and(|prefix| effect.starts_with(prefix)) =>
                    {
                        Step::Boundary
                    }
                    _ => Step::Continue,
                }
            } else {
                let mut entries = Vec::new();
                let mut values = HashSet::new();
                for (index, (branch, body)) in branches.into_iter().enumerate() {
                    let (subpath, label, choice) = match command {
                        Command::Choice { options } => (
                            format!("{path}.options[{index}]"),
                            options[index].text.clone(),
                            true,
                        ),
                        Command::Switch { flag, cases, .. } => {
                            let label = if let Some(case) = cases.get(index) {
                                if !values.insert(case.case.value) {
                                    continue;
                                }
                                format!("case {:?}", case.case.value)
                            } else {
                                if flag.is_some() && values.len() == 2 {
                                    continue;
                                }
                                "default".into()
                            };
                            (format!("{path}.{branch}"), label, false)
                        }
                        _ => (format!("{path}.{branch}"), branch.clone(), false),
                    };
                    let first =
                        self.sequence(body, scene, &format!("{path}.{branch}"), next, boundary);
                    entries.push(self.push(
                        scene,
                        subpath,
                        Step::Decision { label, choice },
                        first,
                    ));
                }
                Step::Branch(entries)
            };
            next = self.push(scene, path, step, next);
        }
        next
    }

    fn missing(
        &self,
        stat: Option<&str>,
        observes: impl Fn(&Node<'_>, &&str) -> bool,
        covers: impl Fn(&Node<'_>) -> bool,
    ) -> Vec<DialogueWarning> {
        let mut pending: VecDeque<_> = self
            .starts
            .iter()
            .map(|&id| (id, Trace::default()))
            .collect();
        let mut visited = HashSet::new();
        let mut reported = HashSet::new();
        let mut warnings = Vec::new();
        while let Some((id, mut trace)) = pending.pop_front() {
            // Identical states at a join share the rest of their work. Loops cannot grow the trace.
            if !visited.insert((id, trace)) {
                continue;
            }
            let node = &self.nodes[id];
            if !trace.covered && covers(node) {
                trace = Trace {
                    covered: true,
                    ..Trace::default()
                };
            }
            match &node.step {
                Step::End | Step::Boundary => {
                    if let Some(appearance) = trace.appearance {
                        let key = (
                            trace.choice,
                            trace.condition,
                            trace.choice.or(trace.condition).unwrap_or(appearance),
                        );
                        if reported.insert(key) {
                            let appeared = &self.nodes[appearance];
                            let Step::Text(speaker) = appeared.step else {
                                unreachable!()
                            };
                            warnings.push(DialogueWarning {
                                speaker: speaker.into(),
                                stat: stat.map(str::to_owned),
                                appearance: appeared.site.clone(),
                                choice: self.decision(trace.choice),
                                condition: self.decision(trace.condition),
                            });
                        }
                    }
                    if matches!(node.step, Step::Boundary) {
                        pending.push_back((node.next, Trace::default()));
                    }
                    continue;
                }
                Step::Go(target) => {
                    pending.push_back((self.entries[target], trace));
                    continue;
                }
                Step::Branch(entries) => {
                    pending.extend(entries.iter().map(|&entry| (entry, trace)));
                    continue;
                }
                Step::Text(speaker) if !trace.covered && observes(node, speaker) => {
                    trace.appearance.get_or_insert(id);
                }
                Step::Decision { choice, .. } if !trace.covered => {
                    if *choice {
                        trace.choice = Some(id);
                        trace.condition = None;
                    } else {
                        trace.condition = Some(id);
                    }
                }
                _ => {}
            }
            pending.push_back((node.next, trace));
        }
        warnings
    }

    fn decision(&self, id: Option<usize>) -> Option<SelectedBranch> {
        let node = &self.nodes[id?];
        let Step::Decision { label, .. } = &node.step else {
            return None;
        };
        Some(SelectedBranch {
            site: node.site.clone(),
            label: label.clone(),
        })
    }
}
