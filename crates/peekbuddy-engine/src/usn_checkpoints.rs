//! The USN source's checkpoints kept by the store (#260).
//!
//! [`RootCheckpoints`] reads the saved position of a root from the pipeline
//! and queues every new one behind the rows it covers; the logic and its
//! tests live in `peekbuddy-store`, which builds on Linux. A runner gets
//! `Box::new(RootCheckpoints::new(store.producer(), root))`.

use peekbuddy_core::usn::checkpoint::Checkpoint;
pub use peekbuddy_store::checkpoints::RootCheckpoints;

use crate::usn_source::Checkpoints;

impl Checkpoints for RootCheckpoints {
    fn load(&self) -> Option<Checkpoint> {
        self.saved()
    }

    fn save(&self, checkpoint: Checkpoint) {
        self.record(checkpoint);
    }
}
