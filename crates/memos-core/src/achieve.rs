//! Milestones: what counts as an achievement, and when one has just happened.
//!
//! Deliberately pure. This module holds the entire opinion about what is worth
//! congratulating someone for, and it decides it from four numbers and a set of
//! codes already awarded — no database, no clock, no I/O. That is what makes
//! the ladder testable, and it is what stops the rules from being spread across
//! the query that reads the counters and the view that draws the badge.
//!
//! The single invariant: a milestone fires **once, ever**. Every rung has a
//! stable `code`, the caller passes in the codes already earned, and nothing
//! already in that set is returned again. Storage enforces it a second time
//! with a primary key, because an achievement announced twice is worse than one
//! never announced — the first is a bug the user can see.

use std::collections::HashSet;

/// The counters a rung can be measured against.
///
/// Lifetime figures, not this week's. A weekly allowance is a billing concept;
/// what someone has built up is the thing worth marking, and it must not go
/// down when Monday arrives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub words: u64,
    pub captures: u64,
    pub commands: u64,
    /// Memories currently held. Falls when things are deleted, which is why it
    /// is not what the capture ladder is measured against.
    pub items: u32,
    pub collections: u32,
    pub tasks_done: u32,
    /// Days on which anything at all was said.
    pub days_active: u32,
    pub speech_ms: u64,
}

impl Totals {
    /// Words per minute of speech, or `None` before there is enough to average.
    ///
    /// Under a minute of total speech the figure swings wildly between
    /// captures, and a number that changes by forty between two sentences
    /// reads as broken rather than as precise.
    pub fn wpm(&self) -> Option<u32> {
        if self.speech_ms < 60_000 || self.words == 0 {
            return None;
        }
        Some((self.words as f64 / (self.speech_ms as f64 / 60_000.0)).round() as u32)
    }
}

/// Days, as the user lived them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Streak {
    /// Consecutive active days ending today (or yesterday, if today is quiet —
    /// a streak is not broken until a day has actually been missed).
    pub current: u32,
    pub longest: u32,
    /// The most words ever said in a single day.
    pub best_day: u32,
    /// How many days were quiet before today's first command. Zero on a day
    /// that follows an active one; large after a fortnight away.
    pub quiet_before: u32,
}

/// A week worth summarising, handed in only on the day the summary is due.
///
/// The caller decides when that is, because "is it Monday for this user" is a
/// question about a clock and a time zone, and neither belongs in here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recap {
    /// ISO week the summary covers, e.g. `2026-W36`. It is part of the code,
    /// so a given week can only ever be summarised once.
    pub iso_week: String,
    pub words: u64,
    pub captures: u64,
    pub days: u32,
}

/// What a rung is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Words,
    Captures,
    TasksDone,
    Collections,
    Streak,
    /// Words in a single day.
    BestDay,
    /// Days active, whether or not consecutive.
    DaysActive,
}

/// Why a notification exists, which is also how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something the user did. Congratulatory.
    Milestone,
    /// Something the user might not know about. Informative, and dismissible.
    Nudge,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Milestone => "milestone",
            Kind::Nudge => "nudge",
        }
    }
}

/// One rung of the ladder.
struct Rung {
    code: &'static str,
    trigger: Trigger,
    at: u64,
    /// `{n}` is replaced with the threshold, grouped: `7,777`.
    title: &'static str,
    body: &'static str,
}

/// The ladder.
///
/// The gaps widen as they go, on purpose. Early rungs are close together
/// because the first week is when someone decides whether this is a habit;
/// later ones are far apart because a badge that arrives every other day stops
/// being a reward and becomes a notification to mute.
const LADDER: &[Rung] = &[
    // ------------------------------------------------------------- words
    Rung {
        code: "words-100",
        trigger: Trigger::Words,
        at: 100,
        title: "Your first {n} words",
        body: "That is a page of notes you did not have to type.",
    },
    Rung {
        code: "words-500",
        trigger: Trigger::Words,
        at: 500,
        title: "{n} words dictated",
        body: "Roughly ten minutes of typing, saved.",
    },
    Rung {
        code: "words-1000",
        trigger: Trigger::Words,
        at: 1_000,
        title: "You just crossed {n} words!",
        body: "Four figures. It only goes up from here.",
    },
    Rung {
        code: "words-2500",
        trigger: Trigger::Words,
        at: 2_500,
        title: "{n} words dictated",
        body: "About half an hour of typing you never did.",
    },
    Rung {
        code: "words-5000",
        trigger: Trigger::Words,
        at: 5_000,
        title: "{n} words dictated",
        body: "Five thousand. That is a long essay, spoken.",
    },
    Rung {
        code: "words-7777",
        trigger: Trigger::Words,
        at: 7_777,
        title: "You just crossed {n} words!",
        body: "Lucky avalanche - keep it rolling.",
    },
    Rung {
        code: "words-10000",
        trigger: Trigger::Words,
        at: 10_000,
        title: "{n} words",
        body: "Five figures. A short book's worth of thinking out loud.",
    },
    Rung {
        code: "words-25000",
        trigger: Trigger::Words,
        at: 25_000,
        title: "{n} words dictated",
        body: "At this point the keyboard is the slow way round.",
    },
    Rung {
        code: "words-50000",
        trigger: Trigger::Words,
        at: 50_000,
        title: "{n} words",
        body: "A novel, near enough. Spoken, not typed.",
    },
    Rung {
        code: "words-100000",
        trigger: Trigger::Words,
        at: 100_000,
        title: "{n} words",
        body: "Six figures. Very few people get here.",
    },
    // ---------------------------------------------------------- captures
    Rung {
        code: "captures-1",
        trigger: Trigger::Captures,
        at: 1,
        title: "Your first memory is saved",
        body: "It is searchable already - try asking for it by what it was about.",
    },
    Rung {
        code: "captures-10",
        trigger: Trigger::Captures,
        at: 10,
        title: "{n} memories saved",
        body: "Enough that search starts earning its keep.",
    },
    Rung {
        code: "captures-50",
        trigger: Trigger::Captures,
        at: 50,
        title: "{n} memories saved",
        body: "A library rather than a list.",
    },
    Rung {
        code: "captures-100",
        trigger: Trigger::Captures,
        at: 100,
        title: "{n} memories saved",
        body: "Three digits. You are outsourcing your memory properly now.",
    },
    Rung {
        code: "captures-500",
        trigger: Trigger::Captures,
        at: 500,
        title: "{n} memories saved",
        body: "Most of what you have thought about this year is in here.",
    },
    Rung {
        code: "captures-1000",
        trigger: Trigger::Captures,
        at: 1_000,
        title: "{n} memories saved",
        body: "A thousand. This is a second brain by any honest definition.",
    },
    // ------------------------------------------------------------- tasks
    Rung {
        code: "tasks-1",
        trigger: Trigger::TasksDone,
        at: 1,
        title: "First task ticked off",
        body: "Spoken into existence, then done.",
    },
    Rung {
        code: "tasks-10",
        trigger: Trigger::TasksDone,
        at: 10,
        title: "{n} tasks finished",
        body: "Ten things that did not fall through the cracks.",
    },
    Rung {
        code: "tasks-50",
        trigger: Trigger::TasksDone,
        at: 50,
        title: "{n} tasks finished",
        body: "You are clearing them faster than you speak them.",
    },
    Rung {
        code: "tasks-100",
        trigger: Trigger::TasksDone,
        at: 100,
        title: "{n} tasks finished",
        body: "A hundred. That is a quarter's worth of loose ends.",
    },
    // ------------------------------------------------------- collections
    Rung {
        code: "collections-3",
        trigger: Trigger::Collections,
        at: 3,
        title: "{n} collections",
        body: "Shape is starting to appear in what you save.",
    },
    Rung {
        code: "collections-10",
        trigger: Trigger::Collections,
        at: 10,
        title: "{n} collections",
        body: "Filed properly - and nothing is trapped in one folder.",
    },
    Rung {
        code: "collections-25",
        trigger: Trigger::Collections,
        at: 25,
        title: "{n} collections",
        body: "This is a structure now, not a pile.",
    },
    // ----------------------------------------------------------- streaks
    Rung {
        code: "streak-3",
        trigger: Trigger::Streak,
        at: 3,
        title: "{n} days in a row",
        body: "Three days is where a habit starts to hold.",
    },
    Rung {
        code: "streak-7",
        trigger: Trigger::Streak,
        at: 7,
        title: "{n} day streak",
        body: "A full week. Not one day missed.",
    },
    Rung {
        code: "streak-14",
        trigger: Trigger::Streak,
        at: 14,
        title: "{n} day streak",
        body: "A fortnight. This is just how you work now.",
    },
    Rung {
        code: "streak-30",
        trigger: Trigger::Streak,
        at: 30,
        title: "{n} day streak",
        body: "A month unbroken. Genuinely rare.",
    },
    Rung {
        code: "streak-100",
        trigger: Trigger::Streak,
        at: 100,
        title: "{n} day streak",
        body: "One hundred days. Nothing left to prove.",
    },
    // ---------------------------------------------------------- best day
    Rung {
        code: "bestday-500",
        trigger: Trigger::BestDay,
        at: 500,
        title: "{n} words in one day",
        body: "Your busiest day so far.",
    },
    Rung {
        code: "bestday-1500",
        trigger: Trigger::BestDay,
        at: 1_500,
        title: "{n} words in a single day",
        body: "That is a heavy day's writing, done out loud.",
    },
    Rung {
        code: "bestday-3000",
        trigger: Trigger::BestDay,
        at: 3_000,
        title: "{n} words in one day",
        body: "A personal record that will be hard to beat.",
    },
    // -------------------------------------------------------- days active
    Rung {
        code: "days-7",
        trigger: Trigger::DaysActive,
        at: 7,
        title: "{n} days of dictating",
        body: "A week of use, however it was spread.",
    },
    Rung {
        code: "days-30",
        trigger: Trigger::DaysActive,
        at: 30,
        title: "{n} days of dictating",
        body: "A month of days with something worth saying.",
    },
    Rung {
        code: "days-100",
        trigger: Trigger::DaysActive,
        at: 100,
        title: "{n} days of dictating",
        body: "A hundred days. This has outlasted most apps on the machine.",
    },
];

/// Nudges: things worth saying once, when the moment is right.
///
/// Separate from the ladder because they are not achievements — nobody earned
/// them — and because each has its own condition rather than a threshold. They
/// are held to the same once-ever rule, and every one of them is dismissible.
struct Nudge {
    code: &'static str,
    title: &'static str,
    body: &'static str,
    goto: Option<&'static str>,
}

const NUDGE_SHORTCUTS: Nudge = Nudge {
    code: "nudge-shortcuts",
    title: "Know the shortcuts?",
    body: "Help, at the foot of the sidebar, lists every chord - including the one \
           you have bound for capture.",
    goto: None,
};

const NUDGE_COLLECTIONS: Nudge = Nudge {
    code: "nudge-collections",
    title: "Say where things should go",
    body: "Saying \"save this to Work\" files a memory as you speak it. Collections \
           keep the library findable once it grows.",
    goto: Some("collections"),
};

const NUDGE_REFER: Nudge = Nudge {
    code: "nudge-refer",
    title: "Give a month, get a month",
    body: "You are getting real use out of this. Refer someone and you both get a \
           month of Pro.",
    goto: Some("referral"),
};

const NUDGE_COMEBACK: Nudge = Nudge {
    code: "nudge-comeback",
    title: "Welcome back",
    body: "Everything you saved is still here, and still searchable.",
    goto: Some("library"),
};

/// A notification that has just become true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earned {
    /// Stable and unique. The storage key, and the once-ever guard.
    pub code: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    /// Which page to open when it is clicked, if any.
    pub goto: Option<&'static str>,
    /// The figure that crossed the line, for the record.
    pub progress: u64,
}

/// Everything newly true, given where the user is and what they have already
/// been told.
///
/// Order matters on the way out: milestones before nudges, and within
/// milestones, smallest first. Crossing three rungs at once (an import, a long
/// first session) should read as a climb, not as an unordered pile.
pub fn earned(
    totals: &Totals,
    streak: &Streak,
    recap: Option<&Recap>,
    already: &HashSet<String>,
) -> Vec<Earned> {
    let mut out = Vec::new();

    for rung in LADDER {
        let value = match rung.trigger {
            Trigger::Words => totals.words,
            Trigger::Captures => totals.captures,
            Trigger::TasksDone => totals.tasks_done as u64,
            Trigger::Collections => totals.collections as u64,
            Trigger::Streak => streak.current as u64,
            Trigger::BestDay => streak.best_day as u64,
            Trigger::DaysActive => totals.days_active as u64,
        };
        if value < rung.at || already.contains(rung.code) {
            continue;
        }
        out.push(Earned {
            code: rung.code.to_string(),
            kind: Kind::Milestone,
            title: rung.title.replace("{n}", &grouped(rung.at)),
            body: rung.body.to_string(),
            goto: None,
            // The threshold, not the current value. The badge is a record of
            // the line that was crossed; the live figure is on the Home page
            // and will have moved on by tomorrow.
            progress: rung.at,
        });
    }

    // A gap long enough that the app has stopped being part of the day. Seven
    // days rather than two: a weekend away is not a lapse, and greeting
    // somebody back from one is the app noticing far too much.
    if streak.quiet_before >= 7 && totals.commands > 0 {
        push_nudge(&mut out, &NUDGE_COMEBACK, already, streak.quiet_before as u64);
    }

    // Once there is enough here to be worth organising, and not before —
    // suggesting collections to somebody with four memories is advice about a
    // problem they do not have.
    if totals.captures >= 15 && totals.collections <= 1 {
        push_nudge(&mut out, &NUDGE_COLLECTIONS, already, totals.captures);
    }

    // After the second day, by which point the shortcut is the thing they are
    // using and the rest of the app is still unexplored.
    if totals.days_active >= 2 {
        push_nudge(&mut out, &NUDGE_SHORTCUTS, already, totals.days_active as u64);
    }

    // Only to someone who is genuinely getting value out of it. Asking for a
    // referral on day one is asking a stranger to vouch for you.
    if streak.current >= 3 && totals.words >= 1_000 {
        push_nudge(&mut out, &NUDGE_REFER, already, totals.words);
    }

    if let Some(r) = recap {
        let code = format!("recap-{}", r.iso_week);
        if !already.contains(&code) && r.captures > 0 {
            out.push(Earned {
                code,
                kind: Kind::Nudge,
                title: "Last week, in numbers".to_string(),
                body: format!(
                    "{} words across {} {} on {} {}.",
                    grouped(r.words),
                    grouped(r.captures),
                    plural(r.captures, "memory", "memories"),
                    grouped(r.days as u64),
                    plural(r.days as u64, "day", "days"),
                ),
                goto: Some("home"),
                progress: r.words,
            });
        }
    }

    out
}

fn push_nudge(out: &mut Vec<Earned>, n: &Nudge, already: &HashSet<String>, progress: u64) {
    if already.contains(n.code) {
        return;
    }
    out.push(Earned {
        code: n.code.to_string(),
        kind: Kind::Nudge,
        title: n.title.to_string(),
        body: n.body.to_string(),
        goto: n.goto,
        progress,
    });
}

fn plural(n: u64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// `7777` becomes `7,777`.
///
/// Written by hand rather than pulled from a formatting crate: this is the one
/// place in the workspace that needs it, and the alternative is a dependency
/// with a locale database attached to it.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(codes: &[&str]) -> HashSet<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    fn has(out: &[Earned], code: &str) -> bool {
        out.iter().any(|e| e.code == code)
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(7_777), "7,777");
        assert_eq!(grouped(100_000), "100,000");
    }

    #[test]
    fn a_rung_fires_once_and_only_once() {
        let totals = Totals {
            words: 1_200,
            commands: 4,
            ..Default::default()
        };
        let streak = Streak::default();

        let first = earned(&totals, &streak, None, &HashSet::new());
        assert!(has(&first, "words-1000"));

        // Told once, never again — even though the total is still above it.
        let already: HashSet<String> = first.iter().map(|e| e.code.clone()).collect();
        let second = earned(&totals, &streak, None, &already);
        assert!(second.is_empty(), "re-fired: {second:?}");
    }

    #[test]
    fn several_rungs_at_once_arrive_smallest_first() {
        let totals = Totals {
            words: 6_000,
            commands: 20,
            ..Default::default()
        };
        let got: Vec<_> = earned(&totals, &Streak::default(), None, &HashSet::new())
            .into_iter()
            .filter(|e| e.code.starts_with("words-"))
            .map(|e| e.progress)
            .collect();
        assert_eq!(got, vec![100, 500, 1_000, 2_500, 5_000]);
    }

    #[test]
    fn the_title_carries_the_number_it_crossed() {
        let totals = Totals {
            words: 8_000,
            ..Default::default()
        };
        let lucky = earned(&totals, &Streak::default(), None, &set(&[]))
            .into_iter()
            .find(|e| e.code == "words-7777")
            .expect("7,777 crossed");
        assert_eq!(lucky.title, "You just crossed 7,777 words!");
    }

    #[test]
    fn a_comeback_needs_a_real_absence() {
        let totals = Totals {
            commands: 1,
            ..Default::default()
        };
        let short = Streak {
            quiet_before: 2,
            ..Default::default()
        };
        assert!(!has(&earned(&totals, &short, None, &set(&[])), "nudge-comeback"));

        let long = Streak {
            quiet_before: 21,
            ..Default::default()
        };
        assert!(has(&earned(&totals, &long, None, &set(&[])), "nudge-comeback"));
    }

    #[test]
    fn the_referral_nudge_waits_for_real_use() {
        let keen = Totals {
            words: 4_000,
            commands: 30,
            days_active: 4,
            ..Default::default()
        };
        let cold = Streak {
            current: 1,
            ..Default::default()
        };
        assert!(!has(&earned(&keen, &cold, None, &set(&[])), "nudge-refer"));

        let warm = Streak {
            current: 5,
            ..Default::default()
        };
        assert!(has(&earned(&keen, &warm, None, &set(&[])), "nudge-refer"));
    }

    #[test]
    fn a_week_is_summarised_once() {
        let recap = Recap {
            iso_week: "2026-W36".into(),
            words: 4_210,
            captures: 31,
            days: 5,
        };
        let totals = Totals::default();
        let out = earned(&totals, &Streak::default(), Some(&recap), &set(&[]));
        let summary = out
            .iter()
            .find(|e| e.code == "recap-2026-W36")
            .expect("a recap for that week");
        assert_eq!(summary.body, "4,210 words across 31 memories on 5 days.");

        let again = earned(
            &totals,
            &Streak::default(),
            Some(&recap),
            &set(&["recap-2026-W36"]),
        );
        assert!(!has(&again, "recap-2026-W36"));
    }

    #[test]
    fn words_per_minute_waits_for_a_minute_of_speech() {
        let thin = Totals {
            words: 40,
            speech_ms: 12_000,
            ..Default::default()
        };
        assert_eq!(thin.wpm(), None);

        let enough = Totals {
            words: 300,
            speech_ms: 180_000,
            ..Default::default()
        };
        assert_eq!(enough.wpm(), Some(100));
    }
}
