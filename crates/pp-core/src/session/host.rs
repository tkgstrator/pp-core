//! Values the host game owns and copies into the scenario between scenes.

use super::Session;

impl Session {
    /// Overwrite a stat owned by the host game, such as a value from a separate
    /// training simulation. The value is clamped to the configured bounds and is
    /// not reported as a script state change.
    pub fn set_stat(&mut self, id: &str, value: i64) -> Result<(), String> {
        self.state
            .update_stat(&self.scenario, id, |_| i128::from(value))?;
        self.state.changes.clear();
        Ok(())
    }

    /// Overwrite a flag owned by the host game without recording a script change.
    pub fn set_flag(&mut self, id: &str, value: bool) -> Result<(), String> {
        self.state.set_flag(id, value)?;
        self.state.changes.clear();
        Ok(())
    }
}
