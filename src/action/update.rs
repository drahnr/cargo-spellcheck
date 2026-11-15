//! Interface for applying deferred source changes at session commit.

use super::{interactive::UserPicked, Action};
use crate::errors::Result;

pub(super) trait SourceUpdates {
    fn apply_source_changes(&mut self, changes: UserPicked) -> Result<()>;
}

impl SourceUpdates for Action {
    fn apply_source_changes(&mut self, changes: UserPicked) -> Result<()> {
        self.write_user_pick_changes_to_disk(changes)
    }
}
