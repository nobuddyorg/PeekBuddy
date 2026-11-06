//! Tracy plots of the writer thread (#163), compiled only with the `tracy`
//! feature of the profiling build.
//!
//! They are sent once per committed batch from the commit observer, which runs
//! on the writer thread: nothing here is on the capture path (no `tracing`
//! macro, no allocation per event, F10).

use peekbuddy_core::perf::CounterSnapshot;
use peekbuddy_store::pipeline::CommitInfo;
use tracy_client::{Client, plot_name};

/// A frame mark per batch (the secondary frame set "batch") and the plots:
/// batch size, ring depth, a replay from the spill, overflows and gaps so far.
pub fn commit(info: &CommitInfo, snapshot: &CounterSnapshot) {
    let Some(client) = Client::running() else {
        return;
    };
    client.secondary_frame_mark(tracy_client::frame_name!("batch"));
    // Counts to a float for a plot: the values are far below 2^53.
    #[expect(
        clippy::cast_precision_loss,
        reason = "plot values: counts far below 2^53"
    )]
    let plot = |name: tracy_client::PlotName, value: u64| client.plot(name, value as f64);
    plot(plot_name!("batch size"), info.commit.rows);
    plot(
        plot_name!("ring depth"),
        u64::try_from(info.ring_depth).unwrap_or(u64::MAX),
    );
    plot(plot_name!("spill replay"), u64::from(info.from_spill));
    plot(plot_name!("overflows"), snapshot.overflows);
    plot(plot_name!("gaps"), snapshot.gaps);
}
