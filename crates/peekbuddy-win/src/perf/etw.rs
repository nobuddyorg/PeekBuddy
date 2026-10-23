//! The `NoBuddy-PeekBuddy` TraceLogging provider (#21).
//!
//! Compiles on every platform; off Windows every write is a no-op. Events
//! are diagnostics only: ETW can drop events under pressure, so it is never a
//! data path.

use std::sync::atomic::{AtomicBool, Ordering};

use peekbuddy_core::perf::{CounterSnapshot, Milestone, Role};
use tracelogging as tlg;

/// Provider name; PerfView and WPR also accept `*NoBuddy-PeekBuddy`.
pub const PROVIDER_NAME: &str = "NoBuddy-PeekBuddy";
/// Provider GUID: the standard name hash of [`PROVIDER_NAME`]. Never change
/// it; WPR profiles and saved traces refer to it.
pub const PROVIDER_GUID: &str = "69eb5350-8420-5c03-5c05-1a6dafc0df73";
/// Keyword for milestone events.
pub const KEYWORD_MILESTONE: u64 = 0x1;
/// Keyword for counter events.
pub const KEYWORD_COUNTER: u64 = 0x2;

tlg::define_provider!(
    PROVIDER,
    "NoBuddy-PeekBuddy",
    id("69eb5350-8420-5c03-5c05-1a6dafc0df73")
);

static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Keeps the provider registered; unregisters on drop, which covers every
/// exit path that unwinds or returns from `main` (the engine also drops it
/// explicitly on `WM_ENDSESSION`, #59).
#[derive(Debug)]
pub struct Registration {
    owner: bool,
}

/// Registers the provider. A second call while a [`Registration`] is alive
/// returns a guard that does nothing.
#[must_use]
pub fn register() -> Registration {
    let owner = !REGISTERED.swap(true, Ordering::AcqRel);
    if owner {
        // SAFETY: PeekBuddy is an exe, never a DLL, so no module unload can
        // happen while the callback is registered; the `REGISTERED` flag
        // ensures register/unregister are never called twice concurrently.
        let _ = unsafe { PROVIDER.register() };
    }
    Registration { owner }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if self.owner {
            PROVIDER.unregister();
            REGISTERED.store(false, Ordering::Release);
        }
    }
}

/// Whether a session is listening for milestones.
#[must_use]
pub fn milestones_enabled() -> bool {
    PROVIDER.enabled(tlg::Level::Informational, KEYWORD_MILESTONE)
}

/// Writes one milestone event. Never allocates.
pub fn milestone(milestone: Milestone, role: Role, since_process_create_us: u64) {
    tlg::write_event!(
        PROVIDER,
        "Milestone",
        level(Informational),
        keyword(0x1),
        str8("Name", milestone.name()),
        str8("Role", role.as_str()),
        u64("SinceProcessCreateUs", &since_process_create_us),
    );
}

/// Writes one counter event (piggyback on batch commit, never a timer).
pub fn counters(snapshot: &CounterSnapshot) {
    tlg::write_event!(
        PROVIDER,
        "Counters",
        level(Informational),
        keyword(0x2),
        u64("EventsReceived", &snapshot.events_received),
        u64("RowsPersisted", &snapshot.rows_persisted),
        u64("Overflows", &snapshot.overflows),
        u64("Gaps", &snapshot.gaps),
        u64("RingDepthHighWater", &snapshot.ring_depth_high_water),
        u64("LastBatchSize", &snapshot.last_batch_size),
    );
}

#[cfg(test)]
mod tests {
    use super::{PROVIDER, PROVIDER_GUID, PROVIDER_NAME, register};
    use tracelogging::Guid;

    #[test]
    fn guid_never_changes() {
        assert_eq!(PROVIDER_GUID, "69eb5350-8420-5c03-5c05-1a6dafc0df73");
        let id = PROVIDER.id().to_utf8_bytes();
        assert_eq!(std::str::from_utf8(&id).unwrap(), PROVIDER_GUID);
    }

    #[test]
    fn guid_is_the_name_hash() {
        let hashed = Guid::from_name(PROVIDER_NAME).to_utf8_bytes();
        assert_eq!(std::str::from_utf8(&hashed).unwrap(), PROVIDER_GUID);
        assert_eq!(PROVIDER.name(), PROVIDER_NAME);
    }

    #[test]
    fn nested_registration_is_harmless() {
        let outer = register();
        let inner = register();
        assert!(outer.owner);
        assert!(!inner.owner);
        drop(inner);
        drop(outer);
        let again = register();
        assert!(again.owner);
    }
}
