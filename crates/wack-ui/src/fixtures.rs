//! Sample data for gallery scenes: a believable WackCode session, so a scene shows real lengths
//! and real rhythm rather than lorem ipsum.

use crate::{
    composer::ComposerData,
    shell::ProjectInfo,
    sidebar::{ChatRow, ChatState, ProjectRow, SidebarData},
    transcript::{
        Block, Diff, DiffLine, Item, LineKind, Meta, Span, Target, Token, Work, WorkRow,
    },
};

/// Which chat the sidebar shows as selected.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    None,
    Finished,
    Working,
}

pub fn sidebar(selected: Selected) -> SidebarData {
    let chat = |title: &'static str, age: &'static str, state, this: Selected| ChatRow {
        title: title.into(),
        age: age.into(),
        state,
        selected: selected == this && this != Selected::None,
    };
    let closed = |name: &'static str, chats: usize| ProjectRow {
        name: name.into(),
        chats: (0..chats)
            .map(|_| chat("", "", ChatState::Idle, Selected::None))
            .collect(),
        open: false,
    };
    SidebarData {
        pinned: vec![
            ProjectRow {
                name: "wackcode".into(),
                open: true,
                chats: vec![
                    chat("Fix the SSE decoder tail", "now", ChatState::Idle, Selected::Finished),
                    chat("Honour Retry-After", "8s", ChatState::Working, Selected::Working),
                    chat("Port the theme derivation", "2h", ChatState::Idle, Selected::None),
                    chat("Tool search for other hosts", "1d", ChatState::Unread, Selected::None),
                    chat("Kill the whole process group", "3d", ChatState::Idle, Selected::None),
                ],
            },
            closed("wackchatter", 3),
            closed("TokenTrail", 2),
        ],
        projects: vec![
            closed("Media-Logger", 4),
            closed("Godot-Bench", 1),
            closed("godot-terraria", 6),
            closed("cargo-cleaner", 0),
            closed("WackWebsite", 2),
        ],
    }
}

pub fn project() -> ProjectInfo {
    ProjectInfo {
        name: "wackcode".into(),
        path: "~/Documents/GitHub/wackcode".into(),
        branch: Some("3.0-Remake".into()),
    }
}

pub fn composer_empty(roomy: bool) -> ComposerData {
    ComposerData {
        text: None,
        placeholder: "Describe a task, or ask about the code".into(),
        model: "GLM-5.3 Flash".into(),
        effort: "Medium".into(),
        context: None,
        working: false,
        queued: Vec::new(),
        roomy,
    }
}

pub fn composer_after_turn() -> ComposerData {
    ComposerData {
        text: Some("Now make retries honour Retry-After".into()),
        context: Some(0.14),
        ..composer_empty(false)
    }
}

pub fn composer_working() -> ComposerData {
    ComposerData {
        context: Some(0.21),
        working: true,
        queued: vec!["Also cap the wait at 60 seconds, whatever the server asks for.".into()],
        ..composer_empty(false)
    }
}

pub const FINISHED_TITLE: &str = "Fix the SSE decoder tail";
pub const WORKING_TITLE: &str = "Honour Retry-After";

pub fn finished_turn() -> Vec<Item> {
    use Token::{Comment, Keyword, Plain, Title};
    vec![
        Item::Prompt(
            "The SSE decoder drops the last event when a stream ends without a blank line. \
             Fix it and add a test."
                .into(),
        ),
        Item::Work(Work {
            summary: "Worked for 19s".into(),
            live: false,
            open: true,
            rows: vec![
                row(
                    "Thought",
                    Target::Words(
                        "The decoder only emits on a blank line, so a final event with no \
                         terminator never leaves the buffer."
                            .into(),
                    ),
                    Meta::Note("2.1s".into()),
                ),
                row("Read", Target::Code("crates/wack-llm/src/sse.rs".into()), Meta::Note("214 lines".into())),
                row(
                    "Searched",
                    Target::Words("“finish” in crates/wack-llm".into()),
                    Meta::Note("3 results".into()),
                ),
                WorkRow {
                    verb: "Edited",
                    target: Target::Code("crates/wack-llm/src/sse.rs".into()),
                    meta: Meta::Lines { added: 6, removed: 1 },
                    diff: Some(Diff {
                        path: "crates/wack-llm/src/sse.rs".into(),
                        lines: vec![
                            line(LineKind::Context, 88, "    /// Called once the body is exhausted."),
                            line(LineKind::Removed, 89, "    pub fn finish(&mut self) {}"),
                            line(LineKind::Added, 89, "    pub fn finish(&mut self) -> Option<Event> {"),
                            line(LineKind::Added, 90, "        if self.data.is_empty() {"),
                            line(LineKind::Added, 91, "            return None;"),
                            line(LineKind::Added, 92, "        }"),
                            line(LineKind::Added, 93, "        Some(self.take_event())"),
                            line(LineKind::Added, 94, "    }"),
                        ],
                    }),
                },
                row("Ran", Target::Code("cargo test -p wack-llm sse".into()), Meta::Note("6 passed".into())),
            ],
        }),
        Item::Reply(vec![
            Block::Paragraph(vec![
                Span::Text("Fixed. "),
                Span::Strong("The decoder now flushes what it has buffered when the stream ends"),
                Span::Text(", so a final event without its blank line is delivered instead of dropped."),
            ]),
            Block::Bullets(vec![
                vec![
                    Span::Code("Decoder::finish"),
                    Span::Text(" returns the pending event, and "),
                    Span::Code("HttpProvider::stream"),
                    Span::Text(" calls it once the body is exhausted."),
                ],
                vec![Span::Text(
                    "A stream that stops halfway through a field still yields nothing: half an \
                     event is never guessed at.",
                )],
            ]),
            Block::Code {
                language: "rust".into(),
                lines: vec![
                    vec![(Token::Meta, "#[test]")],
                    vec![(Keyword, "fn "), (Title, "flushes_an_unterminated_final_event"), (Plain, "() {")],
                    vec![(Plain, "    "), (Keyword, "let mut "), (Plain, "decoder = Decoder::default();")],
                    vec![(Plain, "    decoder.push("), (Token::String, "b\"data: {\\\"done\\\":true}\""), (Plain, ");")],
                    vec![(Plain, "    "), (Comment, "// No blank line: the stream just ends.")],
                    vec![(Plain, "    assert_eq!(decoder.finish().unwrap().data, "), (Token::String, "\"{\\\"done\\\":true}\""), (Plain, ");")],
                    vec![(Plain, "    assert_eq!(decoder.finish(), "), (Keyword, "None"), (Plain, ");")],
                    vec![(Plain, "}")],
                ],
            },
            Block::Paragraph(vec![Span::Text("All six SSE tests pass, including the "), Span::Code("0"), Span::Text("-byte body case.")]),
        ]),
        Item::Footer("19s  ·  GLM-5.3 Flash  ·  12.4k tokens".into()),
    ]
}

pub fn working_turn() -> Vec<Item> {
    // The turn before, as it looks once finished: work folded to one line.
    let mut items: Vec<Item> = finished_turn()
        .into_iter()
        .map(|item| match item {
            Item::Work(work) => Item::Work(Work { open: false, ..work }),
            other => other,
        })
        .collect();
    items.push(Item::Prompt(
        "Make retries honour Retry-After when the server sends it, in seconds or as a date."
            .into(),
    ));
    items.push(Item::Work(Work {
        summary: "Working  ·  8s".into(),
        live: true,
        open: true,
        rows: vec![
            row(
                "Thought",
                Target::Words(
                    "Retry-After can be a number of seconds or an HTTP date; both have to cap \
                     at the backoff ceiling."
                        .into(),
                ),
                Meta::Note("1.4s".into()),
            ),
            row("Read", Target::Code("crates/wack-llm/src/retry.rs".into()), Meta::Note("142 lines".into())),
            row("Read", Target::Code("crates/wack-llm/src/error.rs".into()), Meta::Note("96 lines".into())),
            row("Edited", Target::Code("crates/wack-llm/src/retry.rs".into()), Meta::Lines { added: 24, removed: 3 }),
            row("Ran", Target::Code("cargo test -p wack-llm retry".into()), Meta::Running),
        ],
    }));
    items.push(Item::Reply(vec![Block::Paragraph(vec![
        Span::Text("Retry-After is now read from both 429 and 503 responses. Running the retry tests to check the date form"),
    ])]));
    items
}

fn row(verb: &'static str, target: Target, meta: Meta) -> WorkRow {
    WorkRow { verb, target, meta, diff: None }
}

fn line(kind: LineKind, number: u32, text: &'static str) -> DiffLine {
    DiffLine { kind, number: Some(number), text: text.into() }
}
