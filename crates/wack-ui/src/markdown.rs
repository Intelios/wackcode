//! Markdown in replies, parsed into the few shapes the transcript draws.
//!
//! CommonMark plus tables, strikethrough and task lists (pulldown-cmark). Raw HTML is shown as
//! text, never interpreted. A reply is parsed again while it streams; an unfinished construct
//! (an open code fence) simply parses as what it is so far.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(u8, Vec<Span>),
    /// `start` is the first number of an ordered list.
    List { start: Option<u64>, items: Vec<Item> },
    Code { language: String, text: String },
    Quote(Vec<Block>),
    Rule,
    Table { header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// `Some` for a task list item: whether it is ticked.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

/// One open container while parsing.
enum Open {
    Paragraph(Vec<Span>),
    Heading(u8, Vec<Span>),
    List(Option<u64>, Vec<Item>),
    Item(Item, Vec<Span>),
    Quote(Vec<Block>),
    Code(String, String),
    Table(Vec<Vec<Span>>, Vec<Vec<Vec<Span>>>, bool),
    Row(Vec<Vec<Span>>),
    Cell(Vec<Span>),
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<String>,
}

pub fn parse(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut stack: Vec<Open> = Vec::new();
    let mut done: Vec<Block> = Vec::new();
    let mut style = Style::default();

    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(tag) => {
                // A block starting inside a list item ends the item's loose text first.
                if !matches!(tag, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. })
                    && let Some(Open::Item(item, spans)) = stack.last_mut()
                    && !spans.is_empty()
                {
                    item.blocks.push(Block::Paragraph(std::mem::take(spans)));
                }
                match tag {
                    Tag::Paragraph => stack.push(Open::Paragraph(Vec::new())),
                    Tag::Heading { level, .. } => stack.push(Open::Heading(heading_level(level), Vec::new())),
                    Tag::BlockQuote(_) => stack.push(Open::Quote(Vec::new())),
                    Tag::CodeBlock(kind) => {
                        let language = match kind {
                            CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_string(),
                            CodeBlockKind::Indented => String::new(),
                        };
                        stack.push(Open::Code(language, String::new()));
                    }
                    Tag::List(start) => stack.push(Open::List(start, Vec::new())),
                    Tag::Item => stack.push(Open::Item(Item::default(), Vec::new())),
                    Tag::Table(_) => stack.push(Open::Table(Vec::new(), Vec::new(), false)),
                    Tag::TableHead => {
                        if let Some(Open::Table(_, _, in_head)) = stack.last_mut() {
                            *in_head = true;
                        }
                        stack.push(Open::Row(Vec::new()));
                    }
                    Tag::TableRow => stack.push(Open::Row(Vec::new())),
                    Tag::TableCell => stack.push(Open::Cell(Vec::new())),
                    Tag::Emphasis => style.italic += 1,
                    Tag::Strong => style.bold += 1,
                    Tag::Strikethrough => style.strike += 1,
                    Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => style.links.push(dest_url.to_string()),
                    _ => {}
                }
            }
            Event::End(end) => match end {
                TagEnd::Emphasis => style.italic = style.italic.saturating_sub(1),
                TagEnd::Strong => style.bold = style.bold.saturating_sub(1),
                TagEnd::Strikethrough => style.strike = style.strike.saturating_sub(1),
                TagEnd::Link | TagEnd::Image => {
                    style.links.pop();
                }
                TagEnd::TableHead => {
                    if let Some(Open::Row(cells)) = stack.pop()
                        && let Some(Open::Table(header, _, in_head)) = stack.last_mut()
                    {
                        *header = cells;
                        *in_head = false;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(Open::Row(cells)) = stack.pop()
                        && let Some(Open::Table(_, rows, _)) = stack.last_mut()
                    {
                        rows.push(cells);
                    }
                }
                TagEnd::TableCell => {
                    if let Some(Open::Cell(spans)) = stack.pop()
                        && let Some(Open::Row(cells)) = stack.last_mut()
                    {
                        cells.push(spans);
                    }
                }
                TagEnd::Item => {
                    if let Some(Open::Item(mut item, spans)) = stack.pop() {
                        if !spans.is_empty() {
                            item.blocks.push(Block::Paragraph(spans));
                        }
                        if let Some(Open::List(_, items)) = stack.last_mut() {
                            items.push(item);
                        }
                    }
                }
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::List(_)
                | TagEnd::Table => {
                    let block = match stack.pop() {
                        Some(Open::Paragraph(spans)) => Block::Paragraph(spans),
                        Some(Open::Heading(level, spans)) => Block::Heading(level, spans),
                        Some(Open::Quote(blocks)) => Block::Quote(blocks),
                        Some(Open::Code(language, text)) => Block::Code { language, text: text.trim_end_matches('\n').to_string() },
                        Some(Open::List(start, items)) => Block::List { start, items },
                        Some(Open::Table(header, rows, _)) => Block::Table { header, rows },
                        _ => continue,
                    };
                    place(&mut stack, &mut done, block);
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some(Open::Code(_, code)) = stack.last_mut() {
                    code.push_str(&text);
                } else {
                    push_span(&mut stack, &mut done, span(&text, &style, false));
                }
            }
            Event::Code(text) => push_span(&mut stack, &mut done, span(&text, &style, true)),
            Event::Html(text) | Event::InlineHtml(text) => push_span(&mut stack, &mut done, span(&text, &style, false)),
            Event::SoftBreak => push_span(&mut stack, &mut done, span(" ", &style, false)),
            Event::HardBreak => push_span(&mut stack, &mut done, span("\n", &style, false)),
            Event::Rule => place(&mut stack, &mut done, Block::Rule),
            Event::TaskListMarker(ticked) => {
                if let Some(Open::Item(item, _)) = stack.last_mut() {
                    item.task = Some(ticked);
                }
            }
            _ => {}
        }
    }
    // Close whatever an unfinished stream left open.
    while let Some(open) = stack.pop() {
        let block = match open {
            Open::Paragraph(spans) => Block::Paragraph(spans),
            Open::Heading(level, spans) => Block::Heading(level, spans),
            Open::Quote(blocks) => Block::Quote(blocks),
            Open::Code(language, text) => Block::Code { language, text },
            Open::List(start, items) => Block::List { start, items },
            Open::Item(mut item, spans) => {
                if !spans.is_empty() {
                    item.blocks.push(Block::Paragraph(spans));
                }
                Block::List { start: None, items: vec![item] }
            }
            Open::Table(header, rows, _) => Block::Table { header, rows },
            Open::Row(_) | Open::Cell(_) => continue,
        };
        place(&mut stack, &mut done, block);
    }
    done
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn span(text: &str, style: &Style, code: bool) -> Span {
    Span {
        text: text.to_string(),
        bold: style.bold > 0,
        italic: style.italic > 0,
        strike: style.strike > 0,
        code,
        link: style.links.last().cloned(),
    }
}

/// Adds a finished block to the container it belongs in.
fn place(stack: &mut [Open], done: &mut Vec<Block>, block: Block) {
    match stack.last_mut() {
        Some(Open::Quote(blocks)) => blocks.push(block),
        Some(Open::Item(item, spans)) => {
            if !spans.is_empty() {
                item.blocks.push(Block::Paragraph(std::mem::take(spans)));
            }
            item.blocks.push(block);
        }
        _ => done.push(block),
    }
}

/// Adds inline text to whatever is collecting it, merging with the previous span when the style
/// matches.
fn push_span(stack: &mut Vec<Open>, done: &mut Vec<Block>, span: Span) {
    let spans = match stack.last_mut() {
        Some(Open::Paragraph(spans) | Open::Heading(_, spans) | Open::Item(_, spans) | Open::Cell(spans)) => spans,
        _ => {
            // Inline text with no paragraph around it (raw HTML at the top level, say).
            stack.push(Open::Paragraph(Vec::new()));
            let Some(Open::Paragraph(spans)) = stack.last_mut() else { unreachable!() };
            spans.push(span);
            if let Some(Open::Paragraph(spans)) = stack.pop() {
                place(stack, done, Block::Paragraph(spans));
            }
            return;
        }
    };
    if let Some(last) = spans.last_mut()
        && last.bold == span.bold
        && last.italic == span.italic
        && last.strike == span.strike
        && last.code == span.code
        && last.link == span.link
    {
        last.text.push_str(&span.text);
    } else {
        spans.push(span);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span { text: text.into(), ..Span::default() }
    }

    #[test]
    fn paragraphs_keep_inline_styles() {
        let blocks = parse("Fixed. **The decoder** now `flushes`, see [docs](https://x.dev).");
        let Block::Paragraph(spans) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(spans[0], plain("Fixed. "));
        assert!(spans[1].bold && spans[1].text == "The decoder");
        assert!(spans[3].code && spans[3].text == "flushes");
        assert_eq!(spans[5].link.as_deref(), Some("https://x.dev"));
    }

    #[test]
    fn lists_nest_and_tight_items_are_paragraphs() {
        let blocks = parse("- one\n- two\n  - inner\n\n3. three\n4. four\n");
        let Block::List { start: None, items } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].blocks, [Block::Paragraph(vec![plain("one")])]);
        assert!(matches!(&items[1].blocks[1], Block::List { items, .. } if items.len() == 1));
        assert!(matches!(&blocks[1], Block::List { start: Some(3), items } if items.len() == 2));
    }

    #[test]
    fn code_fences_keep_their_language_even_unfinished() {
        let blocks = parse("```rust\nfn main() {}\n```\n");
        assert_eq!(blocks[0], Block::Code { language: "rust".into(), text: "fn main() {}".into() });
        let streaming = parse("Look:\n\n```py\nprint(1)\n");
        assert!(matches!(&streaming[1], Block::Code { language, text } if language == "py" && text.contains("print(1)")));
    }

    #[test]
    fn tables_quotes_rules_and_tasks() {
        let blocks = parse("| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted\n\n---\n\n- [x] done\n- [ ] todo\n");
        assert!(matches!(&blocks[0], Block::Table { header, rows } if header.len() == 2 && rows.len() == 1));
        assert!(matches!(&blocks[1], Block::Quote(inner) if inner.len() == 1));
        assert_eq!(blocks[2], Block::Rule);
        let Block::List { items, .. } = &blocks[3] else { panic!() };
        assert_eq!((items[0].task, items[1].task), (Some(true), Some(false)));
    }

    #[test]
    fn html_is_text() {
        let blocks = parse("<script>alert(1)</script>");
        assert!(matches!(&blocks[0], Block::Paragraph(spans) if spans[0].text.contains("<script>")));
    }
}
