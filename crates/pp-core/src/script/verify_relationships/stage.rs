use std::collections::{HashSet, VecDeque};

use super::{Graph, Script, ScriptSite, Step};

impl Script {
    /// Reports an event if no local branch contains protagonist dialogue, thought, or portrait.
    pub fn verify_protagonist_presence(&self) -> Vec<String> {
        let graph = Graph::build(self, None);
        self.scenes()
            .iter()
            .filter(|scene| !graph.has_protagonist(graph.entries[scene.scene.as_str()]))
            .map(|scene| scene.scene.clone())
            .collect()
    }

    /// Reports a marked thought if a path can reach it without the protagonist on the left.
    pub fn verify_thought_portraits(&self, thoughts: &[ScriptSite]) -> Vec<ScriptSite> {
        let thoughts: HashSet<_> = thoughts.iter().collect();
        let graph = Graph::build(self, None);
        let mut pending: VecDeque<_> = graph.starts.iter().map(|&node| (node, false)).collect();
        let mut visited = HashSet::new();
        let mut warnings = Vec::new();
        while let Some((id, mut hero_left)) = pending.pop_front() {
            if !visited.insert((id, hero_left)) {
                continue;
            }
            let node = &graph.nodes[id];
            match &node.step {
                Step::Portrait { left: true, asset } => hero_left = protagonist(*asset),
                Step::Text("protagonist") if !hero_left && thoughts.contains(&node.site) => {
                    warnings.push(node.site.clone());
                }
                Step::Go(target) => {
                    pending.push_back((graph.entries[target], hero_left));
                    continue;
                }
                Step::Branch(children) => {
                    pending.extend(children.iter().map(|&child| (child, hero_left)));
                    continue;
                }
                Step::End => continue,
                _ => {}
            }
            pending.push_back((node.next, hero_left));
        }
        warnings
    }
}

fn protagonist(asset: Option<&str>) -> bool {
    asset.and_then(|asset| asset.split(':').next()) == Some("protagonist")
}

impl Graph<'_> {
    fn has_protagonist(&self, start: usize) -> bool {
        let mut pending = vec![start];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let node = &self.nodes[id];
            match &node.step {
                Step::Text("protagonist") => return true,
                Step::Portrait { asset, .. } if protagonist(*asset) => return true,
                // A jump is a different event; its cast cannot cover this one.
                Step::End | Step::Go(_) => {}
                Step::Branch(children) => pending.extend(children),
                _ => pending.push(node.next),
            }
        }
        false
    }
}
