//! The Chat area's words, as pure functions ported from version 2 with their tests: the
//! sidebar's recency groups (`sidebar-utils.ts`), and the new-chat hero's greeting and idea chips
//! (`chat-hero.ts`). The ideas only fill the composer; nothing is sent until the user does.

use wack_core::clock;

/// A group of the Chat area's sidebar, newest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recency {
    Today,
    Yesterday,
    Week,
    Earlier,
}

impl Recency {
    pub fn label(self) -> &'static str {
        match self {
            Recency::Today => "Today",
            Recency::Yesterday => "Yesterday",
            Recency::Week => "Previous 7 days",
            Recency::Earlier => "Earlier",
        }
    }
}

/// `items` (each with its last activity, milliseconds since 1970) in calendar groups, newest
/// first in each; groups with nothing in them are left out.
pub fn group_by_recency<T>(items: Vec<(u64, T)>, now: u64) -> Vec<(Recency, Vec<T>)> {
    let today = clock::start_of_day(now, 0);
    let yesterday = clock::start_of_day(now, 1);
    let week = clock::start_of_day(now, 7);
    let key = |time: u64| match time {
        time if time >= today => Recency::Today,
        time if time >= yesterday => Recency::Yesterday,
        time if time >= week => Recency::Week,
        _ => Recency::Earlier,
    };
    let mut items = items;
    items.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    let mut groups: Vec<(Recency, Vec<T>)> = Vec::new();
    for (time, item) in items {
        let group = key(time);
        match groups.last_mut() {
            Some((last, list)) if *last == group => list.push(item),
            _ => groups.push((group, vec![item])),
        }
    }
    groups
}

/// The hero's greeting for an hour of the day.
pub fn greeting(hour: u32) -> &'static str {
    match hour {
        0..5 => "Up late",
        5..12 => "Good morning",
        12..18 => "Good afternoon",
        _ => "Good evening",
    }
}

/// One idea chip: its words, and what it puts in the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Idea {
    pub label: &'static str,
    pub prompt: &'static str,
}

pub const IDEAS: [Idea; 12] = [
    Idea { label: "Think something through", prompt: "Help me think through " },
    Idea { label: "Explain it simply", prompt: "Explain like I'm new to it: " },
    Idea { label: "Draft a message", prompt: "Help me write a friendly message to " },
    Idea { label: "Plan my week", prompt: "Help me plan my week. Here's what's on my plate: " },
    Idea { label: "Brainstorm names", prompt: "Brainstorm ten names for " },
    Idea { label: "Compare options", prompt: "Compare these options and help me choose: " },
    Idea { label: "Summarise a page", prompt: "Summarise this page for me: " },
    Idea { label: "Learn something new", prompt: "Teach me the basics of " },
    Idea { label: "Cook with what I have", prompt: "What can I cook with " },
    Idea { label: "Polish my writing", prompt: "Make this read better, keep my voice:\n\n" },
    Idea { label: "Gift ideas", prompt: "Gift ideas for someone who loves " },
    Idea { label: "Settle a debate", prompt: "Settle a friendly debate: " },
];

/// `count` different ideas for one visit, chosen from `seed` the same way every time (version
/// 2's Lehmer generator), as indexes into [`IDEAS`].
pub fn pick_ideas(seed: u64, count: usize) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..IDEAS.len()).collect();
    let mut picked = Vec::new();
    let mut state = seed % 2_147_483_647;
    if state == 0 {
        state = 1;
    }
    while picked.len() < count.min(IDEAS.len()) {
        state = state * 48_271 % 2_147_483_647;
        picked.push(pool.remove((state % pool.len() as u64) as usize));
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chats_fall_into_calendar_groups_newest_first() {
        let now = clock::start_of_day(1_790_000_000_000, 0) + 15 * 3_600_000;
        let today = clock::start_of_day(now, 0);
        let items = vec![
            (today - 10 * 86_400_000, "old"),
            (now - 60_000, "a minute ago"),
            (today + 1, "this morning"),
            (today - 3_600_000, "last night"),
            (today - 3 * 86_400_000, "this week"),
        ];
        let groups = group_by_recency(items, now);
        let labels: Vec<(&str, Vec<&str>)> = groups.iter().map(|(group, items)| (group.label(), items.clone())).collect();
        assert_eq!(
            labels,
            [
                ("Today", vec!["a minute ago", "this morning"]),
                ("Yesterday", vec!["last night"]),
                ("Previous 7 days", vec!["this week"]),
                ("Earlier", vec!["old"]),
            ]
        );
        assert!(group_by_recency(vec![(now, "x")], now).iter().all(|(group, _)| *group == Recency::Today), "empty groups are left out");
    }

    #[test]
    fn the_greeting_follows_the_hour() {
        assert_eq!(greeting(3), "Up late");
        assert_eq!(greeting(5), "Good morning");
        assert_eq!(greeting(12), "Good afternoon");
        assert_eq!(greeting(18), "Good evening");
        assert_eq!(greeting(23), "Good evening");
    }

    #[test]
    fn ideas_are_distinct_and_the_same_for_a_seed() {
        for seed in [0, 1, 7, 99_999, u64::MAX] {
            let picked = pick_ideas(seed, 3);
            assert_eq!(picked.len(), 3);
            let mut unique = picked.clone();
            unique.dedup();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), 3, "{seed}: {picked:?}");
            assert_eq!(picked, pick_ideas(seed, 3));
        }
        assert_eq!(pick_ideas(5, 40).len(), IDEAS.len());
        assert_ne!(pick_ideas(1, 3), pick_ideas(2, 3), "different visits get different chips");
    }
}
