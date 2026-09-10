-- 004_achievements — what the user has built up, and what we told them about it.
--
-- Three tables that between them answer one question the store could not
-- answer before: how much has this person actually done? Captures were counted
-- weekly and thrown away (`usage`), words were never counted at all, and the
-- bell invented its two notifications fresh on every render.

-- One row per day the app was used. Everything that reads like an achievement
-- — a streak, a personal best, "you dictated more this week" — is a query over
-- this table, so none of them need their own counter to drift out of step.
--
-- `day` is a local date, not UTC. A streak is a human claim about days as the
-- person lived them: dictating at 11pm and again at 1am is one day's work in
-- London and two in UTC, and the second answer is the wrong one to show.
CREATE TABLE daily_activity (
    day        TEXT PRIMARY KEY,
    -- Words spoken, counted from the transcript at the moment it is routed.
    -- Not derived from `commands`, which the user is invited to erase
    -- (ADR-0006) — a lifetime total that resets when someone clears their
    -- command log is a lie about their history, not a privacy feature.
    words      INTEGER NOT NULL DEFAULT 0,
    -- Commands that saved something, as opposed to searching or undoing.
    captures   INTEGER NOT NULL DEFAULT 0,
    -- Commands of every kind, which is what words-per-minute is averaged over.
    commands   INTEGER NOT NULL DEFAULT 0,
    -- Milliseconds of speech, so words-per-minute is a measured rate rather
    -- than words divided by a guess.
    speech_ms  INTEGER NOT NULL DEFAULT 0
);

-- Every milestone that has ever been reached, and when.
--
-- The row is the guard: a milestone is awarded by inserting here, and the
-- insert is `OR IGNORE`, so crossing 1 000 words twice — restart, re-import,
-- a second evaluation in the same second — can only ever notify once.
CREATE TABLE achievements (
    code      TEXT PRIMARY KEY,
    -- The value that crossed the threshold, kept for the notification body:
    -- "7,777 words" is worth saying, "you passed a milestone" is not.
    progress  INTEGER NOT NULL DEFAULT 0,
    earned_at TEXT NOT NULL
);
CREATE INDEX idx_achievements_earned ON achievements(earned_at DESC);

-- What the bell has to say, stored rather than derived.
--
-- Derived notifications cannot be read: there is nowhere to write down that
-- you have seen one, so every restart re-announces the same milestone. These
-- persist, they are marked read, and they can be dismissed.
CREATE TABLE notifications (
    id           TEXT PRIMARY KEY,
    -- 'milestone' | 'nudge' | 'alert'. Drives the icon and the filter.
    kind         TEXT NOT NULL,
    -- The achievement that produced it, when one did. Null for nudges.
    code         TEXT,
    title        TEXT NOT NULL,
    body         TEXT NOT NULL,
    -- Where clicking it should land, as a page name. Null means it is only
    -- something to read.
    goto         TEXT,
    created_at   TEXT NOT NULL,
    read_at      TEXT,
    dismissed_at TEXT
);
-- The list query: newest first, dismissed ones gone.
CREATE INDEX idx_notifications_live ON notifications(created_at DESC)
    WHERE dismissed_at IS NULL;
