-- Track which events need re-symbolication when their source maps land.
--
-- Currently `symbolicate_event` silently succeeds when the release is known
-- but no `.map` files are registered yet — a very common race during a
-- deploy where events start flowing before the CI upload finishes. The
-- events are marked 'done' with unsymbolicated frames forever.
--
-- This column records that state so a follow-up task can requeue the
-- affected envelopes when release files arrive. Writing the flag here
-- means the data is already captured when we later wire the requeue; no
-- schema migration is needed at that point.
--
-- Values:
--   NULL           — symbolication wasn't attempted or release is absent
--   'ok'           — symbolication completed (may have been a no-op if no
--                    JS frames)
--   'missing_map'  — release is known, but no matching .map file found
--   'failed'       — unexpected error during symbolication

ALTER TABLE events ADD COLUMN symbolication_state TEXT;

CREATE INDEX IF NOT EXISTS idx_events_sym_pending
    ON events(release, symbolication_state)
    WHERE symbolication_state = 'missing_map';
