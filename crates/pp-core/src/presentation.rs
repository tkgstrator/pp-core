//! Translates executed script commands into display and audio cues.

use crate::script::{Bgm, Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// An action for the renderer, audio player, or playback loop.
///
/// Asset and effect IDs borrow from the script. A host can resolve them against
/// its own asset library and register its own screen effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue<'script> {
    SetBackground {
        asset: &'script str,
    },
    SetPortrait {
        side: Side,
        asset: Option<&'script str>,
    },
    /// Start this track, replacing any track currently playing.
    StartBgm {
        asset: &'script str,
    },
    StopBgm,
    /// Start a sound without pausing script execution.
    PlaySe {
        asset: &'script str,
    },
    /// Start an effect without pausing script execution.
    PlayEffect {
        effect: &'script str,
    },
    /// Resume script execution after this many milliseconds.
    Wait {
        ms: u64,
    },
}

/// Persistent presentation state for a single playback session.
///
/// Reuse this state across scene transitions so backgrounds, portraits, and
/// BGM stay active until a subsequent command changes them.
#[derive(Debug, Default)]
pub struct Presentation {
    background: Option<String>,
    left: Option<String>,
    right: Option<String>,
    bgm: Option<String>,
}

impl Presentation {
    pub fn background(&self) -> Option<&str> {
        self.background.as_deref()
    }

    pub fn portrait(&self, side: Side) -> Option<&str> {
        match side {
            Side::Left => self.left.as_deref(),
            Side::Right => self.right.as_deref(),
        }
    }

    pub fn bgm(&self) -> Option<&str> {
        self.bgm.as_deref()
    }

    /// Apply a command reached by the playback loop. The caller handles
    /// control flow, dialogue, and state mutations.
    pub fn apply<'script>(&mut self, command: &'script Command) -> Option<Cue<'script>> {
        let cue = match command {
            Command::Background { asset } => {
                self.background = Some(asset.clone());
                Cue::SetBackground { asset }
            }
            Command::Left { asset } => {
                self.left.clone_from(asset);
                Cue::SetPortrait {
                    side: Side::Left,
                    asset: asset.as_deref(),
                }
            }
            Command::Right { asset } => {
                self.right.clone_from(asset);
                Cue::SetPortrait {
                    side: Side::Right,
                    asset: asset.as_deref(),
                }
            }
            Command::Bgm(Bgm::Start { asset }) => {
                self.bgm = Some(asset.clone());
                Cue::StartBgm { asset }
            }
            Command::Bgm(Bgm::Stop {}) => {
                self.bgm = None;
                Cue::StopBgm
            }
            Command::Se { asset } => Cue::PlaySe { asset },
            Command::Effect { effect } => Cue::PlayEffect { effect },
            Command::Wait { ms } => Cue::Wait { ms: *ms },
            _ => return None,
        };
        Some(cue)
    }
}
