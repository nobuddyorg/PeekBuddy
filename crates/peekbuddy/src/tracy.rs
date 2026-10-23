//! Tracy profiling (#22), compiled only with the `profile-tracy` feature.
//!
//! The `tracing` spans of the process become Tracy zones. The client is
//! built with `ondemand` (nothing is buffered until a viewer connects) and
//! `only-localhost` (no LAN broadcast, no remote listener).

use tracing_subscriber::layer::SubscriberExt as _;

use crate::Role;

/// Tracy allocation events with call stacks (depth 8) around the counting
/// allocator (#23).
#[cfg(feature = "profile-tracy-alloc")]
#[global_allocator]
static ALLOC: tracy_client::ProfiledAllocator<peekbuddy_alloc::CountingAlloc<std::alloc::System>> =
    tracy_client::ProfiledAllocator::new(peekbuddy_alloc::CountingAlloc::new(std::alloc::System), 8);

/// Starts the Tracy client, routes `tracing` spans to it and opens the
/// zone that covers the role's whole run.
pub fn start(role: &Role) -> tracing::span::EnteredSpan {
    let client = tracy_client::Client::start();
    client.set_thread_name("main");
    // Only fails if another subscriber is already installed, which leaves
    // that one in place: profiling then misses spans but the role still
    // runs.
    let _ = tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(tracing_tracy::TracyLayer::default()),
    );
    match role {
        Role::Engine => tracing::info_span!("engine").entered(),
        Role::Viewer => tracing::info_span!("viewer").entered(),
    }
}
