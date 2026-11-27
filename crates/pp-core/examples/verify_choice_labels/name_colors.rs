use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Default, Deserialize)]
pub struct NameColorAudit {
    names: Vec<String>,
    texts: Vec<Vec<TextSpan>>,
}

#[derive(Deserialize)]
struct TextSpan {
    text: String,
    color: String,
}

impl NameColorAudit {
    /// Indices of source text fields and names whose visible span includes a non-red glyph.
    pub fn warnings(&self) -> Vec<(usize, BTreeSet<&str>)> {
        self.texts
            .iter()
            .enumerate()
            .filter_map(|(index, spans)| {
                let mut text = String::new();
                let mut non_red = Vec::new();
                for span in spans {
                    let start = text.len();
                    text.push_str(&span.text);
                    if start < text.len() && !span.color.eq_ignore_ascii_case("#ff2000") {
                        non_red.push(start..text.len());
                    }
                }
                let names: BTreeSet<_> = self
                    .names
                    .iter()
                    .map(String::as_str)
                    .filter(|name| {
                        !name.is_empty()
                            && text.char_indices().any(|(start, _)| {
                                text[start..].starts_with(name)
                                    && non_red.iter().any(|range| {
                                        range.start < start + name.len() && start < range.end
                                    })
                            })
                    })
                    .collect();
                (!names.is_empty()).then_some((index, names))
            })
            .collect()
    }
}
