//! Background task that publishes a fresh `Event::Sessions` snapshot
//! whenever the set of active transcode sessions changes.
//!
//! Rather than threading a Hub handle through every code path that mutates
//! sessions (the API layer, the reaper, future admin actions), we poll the
//! transcoder once a second and diff the id set. Any change — added,
//! removed, or both — triggers a publish.
//!
//! The latency cost (≤1 s) is acceptable for a dashboard. The benefit is
//! that the transcoder crate stays Hub-agnostic and no future call site
//! can forget to emit.

use std::collections::BTreeSet;
use std::time::Duration;

use chimpflix_transcoder::TranscodeManager;
use tokio::time::sleep;

use crate::events::{Event, Hub, SessionsEvent};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub fn spawn(hub: Hub, transcoder: TranscodeManager) {
    tokio::spawn(async move {
        let mut last: BTreeSet<String> = BTreeSet::new();
        loop {
            sleep(POLL_INTERVAL).await;
            let snapshot = transcoder.list_sessions();
            let current: BTreeSet<String> =
                snapshot.iter().map(|s| s.id.clone()).collect();
            if current != last {
                hub.publish(Event::Sessions(SessionsEvent::snapshot(snapshot)));
                last = current;
            }
        }
    });
}
