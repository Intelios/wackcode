//! What becomes of pictures on their way to the model.
//!
//! The session keeps every picture as it arrived; [`prepare`] runs on each request's copy of the
//! messages and never on the session. Its rules:
//! - A model that does not accept images gets none: tool results' pictures are dropped and their
//!   text says so, and a picture the user attached becomes a line saying it was left out. Decided
//!   per request, since a chat can carry on with another model.
//! - Otherwise the newest message's pictures are always sent: the model has not seen them yet,
//!   and eliding a batch it just asked for is what traps a model in taking it again and again (the
//!   loop version 2 fell into). Older pictures, the user's and the tools' alike, are sent while the
//!   request stays within [`MAX_IMAGES`] and [`MAX_BYTES`]; past that the oldest are elided, each
//!   leaving a line that says what it was and that the model already saw it.
//! - The cut moves only in steps of `BATCH` and only forward: it is a function of the pictures
//!   before the newest message, a list that only ever grows at its end, so between moves the start
//!   of the request is unchanged and the provider's prompt cache holds.
//!
//! [`cap_turn`] keeps the newest message itself within the budget: it runs once, when a turn's
//! results arrive, and what it cuts is cut from the session too, with a line saying so, so the
//! model and the transcript agree on what was returned.
//!
//! Every rule keeps every message and block where it was (an elided user picture becomes a text
//! block in its place), so a tool call and its result stay paired.

use wack_llm::{Block, Image, Message};

/// The most pictures one request carries. Anthropic tightens its size rules past 20.
pub const MAX_IMAGES: usize = 20;
/// The most base64 one request carries, well inside every provider's request limit.
pub const MAX_BYTES: usize = 12 * 1024 * 1024;
/// How many older pictures go at once when the cut moves.
const BATCH: usize = 6;

/// Applies the rules to a request's messages.
pub fn prepare(messages: &mut [Message], accepts_images: bool) {
    if !accepts_images {
        leave_out(messages);
        return;
    }
    let Some((newest, older)) = messages.split_last_mut() else { return };
    let kept: Vec<usize> = pictures(std::slice::from_ref(newest)).collect();
    let sizes: Vec<usize> = pictures(older).collect();
    let mut elide = elided_count(&sizes, kept.len(), kept.iter().sum());
    for block in older.iter_mut().flat_map(|message| message.content.iter_mut()) {
        if elide == 0 {
            break;
        }
        match block {
            Block::ToolResult { content, images, .. } if !images.is_empty() => {
                let here = elide.min(images.len());
                for image in images.drain(..here) {
                    append(content, &elided(image.label.as_deref()));
                }
                elide -= here;
            }
            Block::Image { label, .. } => {
                *block = Block::Text { text: elided_attachment(label.as_deref()) };
                elide -= 1;
            }
            _ => {}
        }
    }
}

/// Keeps one turn's results within the budget, newest pictures last to go: past [`MAX_IMAGES`]
/// or [`MAX_BYTES`] a result keeps its text and loses its later pictures, with a line saying how
/// many and why. Pictures are never cut short of the first.
pub fn cap_turn(results: &mut [Block]) {
    let mut count = 0;
    let mut bytes = 0;
    for block in results {
        let Block::ToolResult { content, images, .. } = block else { continue };
        let mut keep = 0;
        for image in images.iter() {
            if count > 0 && (count + 1 > MAX_IMAGES || bytes + image.data.len() > MAX_BYTES) {
                break;
            }
            count += 1;
            bytes += image.data.len();
            keep += 1;
        }
        let cut = images.len() - keep;
        if cut > 0 {
            images.truncate(keep);
            let noun = if cut == 1 { "picture was" } else { "pictures were" };
            append(
                content,
                &format!("[{cut} more {noun} not kept: one step can return at most {MAX_IMAGES} pictures. Ask for fewer at a time.]"),
            );
        }
    }
}

/// The line an elided tool picture leaves in its result's text.
pub fn elided(label: Option<&str>) -> String {
    match label {
        Some(label) => format!("[Picture elided from this request: you already viewed it ({label}).]"),
        None => "[Picture elided from this request: you already viewed it.]".to_string(),
    }
}

fn elided_attachment(label: Option<&str>) -> String {
    match label {
        Some(label) => format!("[A picture the user attached ({label}) is elided from this request: you already viewed it.]"),
        None => "[A picture the user attached is elided from this request: you already viewed it.]".to_string(),
    }
}

fn leave_out(messages: &mut [Message]) {
    for block in messages.iter_mut().flat_map(|message| message.content.iter_mut()) {
        match block {
            Block::ToolResult { content, images, .. } if !images.is_empty() => {
                let count = images.len();
                images.clear();
                let noun = if count == 1 { "image" } else { "images" };
                append(content, &format!("[{count} {noun} left out: this model does not accept images.]"));
            }
            Block::Image { label, .. } => {
                let text = match label {
                    Some(label) => format!("[A picture the user attached ({label}) is left out: this model does not accept images.]"),
                    None => "[A picture the user attached is left out: this model does not accept images.]".to_string(),
                };
                *block = Block::Text { text };
            }
            _ => {}
        }
    }
}

/// The base64 size of each picture in `messages`, oldest first, the user's and the tools' alike.
fn pictures(messages: &[Message]) -> impl Iterator<Item = usize> + '_ {
    messages.iter().flat_map(|message| &message.content).flat_map(|block| match block {
        Block::ToolResult { images, .. } => images.iter().map(|image: &Image| image.data.len()).collect(),
        Block::Image { data, .. } => vec![data.len()],
        _ => Vec::new(),
    })
}

/// How many of the oldest of `older` to elide so the request fits, with `kept_count` pictures of
/// `kept_bytes` always sent: the fewest that fit, rounded up to a whole `BATCH`.
fn elided_count(older: &[usize], kept_count: usize, kept_bytes: usize) -> usize {
    let mut count = older.len() + kept_count;
    let mut bytes = older.iter().sum::<usize>() + kept_bytes;
    let mut fewest = 0;
    while fewest < older.len() && (count > MAX_IMAGES || bytes > MAX_BYTES) {
        count -= 1;
        bytes -= older[fewest];
        fewest += 1;
    }
    if fewest == 0 { 0 } else { (fewest.div_ceil(BATCH) * BATCH).min(older.len()) }
}

fn append(content: &mut String, line: &str) {
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(line);
}

#[cfg(test)]
mod tests {
    use wack_llm::Role;

    use super::*;

    fn image(data: &str) -> Image {
        Image::new("image/png", data).labelled(format!("shot {data}"))
    }

    fn result(id: &str, pictures: impl IntoIterator<Item = Image>) -> Block {
        Block::ToolResult { tool_use_id: id.into(), content: format!("result {id}"), is_error: false, images: pictures.into_iter().collect() }
    }

    fn turn(blocks: Vec<Block>) -> Message {
        Message { role: Role::User, content: blocks }
    }

    /// One tool-result message per picture, numbered from 0, oldest first.
    fn conversation(pictures: usize) -> Vec<Message> {
        (0..pictures).map(|index| turn(vec![result(&format!("t{index}"), [image(&index.to_string())])])).collect()
    }

    fn sent(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|message| &message.content)
            .flat_map(|block| match block {
                Block::ToolResult { images, .. } => images.iter().map(|image| image.data.clone()).collect(),
                Block::Image { data, .. } => vec![data.clone()],
                _ => Vec::new(),
            })
            .collect()
    }

    fn prepared(mut messages: Vec<Message>) -> Vec<Message> {
        prepare(&mut messages, true);
        messages
    }

    #[test]
    fn up_to_the_limit_everything_is_sent() {
        for total in 0..=MAX_IMAGES {
            assert_eq!(sent(&prepared(conversation(total))).len(), total, "at {total}");
        }
    }

    #[test]
    fn past_the_limit_the_oldest_go_a_batch_at_a_time() {
        for total in MAX_IMAGES + 1..=MAX_IMAGES + BATCH {
            let messages = prepared(conversation(total));
            assert_eq!(sent(&messages).len(), total - BATCH, "at {total}");
            assert_eq!(sent(&messages).first().map(String::as_str), Some("6"));
            let Block::ToolResult { content, .. } = &messages[5].content[0] else { panic!() };
            assert_eq!(content, "result t5\n[Picture elided from this request: you already viewed it (shot 5).]");
        }
        assert_eq!(sent(&prepared(conversation(MAX_IMAGES + BATCH + 1))).len(), MAX_IMAGES + 1 - BATCH);
    }

    #[test]
    fn the_newest_batch_is_never_elided() {
        // The loop: a browser batch of twenty screenshots used to reach the model as eight.
        let mut messages = conversation(10);
        messages.push(turn(vec![result("batch", (0..MAX_IMAGES).map(|index| image(&format!("b{index}"))))]));
        let messages = prepared(messages);
        let Block::ToolResult { images, .. } = &messages[10].content[0] else { panic!() };
        assert_eq!(images.len(), MAX_IMAGES);
        assert_eq!(sent(&messages).len(), MAX_IMAGES, "all ten older ones go to make room");
    }

    #[test]
    fn a_second_batch_does_not_cut_into_itself() {
        let five = |name: &str| turn((0..5).map(|index| result(&format!("{name}{index}"), [image(&format!("{name}{index}"))])).collect());
        let mut messages: Vec<Message> = (0..4).map(|round| five(&format!("r{round}"))).collect();
        messages.push(five("new"));
        let messages = prepared(messages);
        let newest = sent(&messages[4..]);
        assert_eq!(newest, ["new0", "new1", "new2", "new3", "new4"]);
        assert!(sent(&messages).len() <= MAX_IMAGES);
    }

    #[test]
    fn the_cut_never_moves_back_and_holds_between_moves() {
        let mut elided_before = 0;
        let mut previous: Option<Vec<Message>> = None;
        for total in 1..=60 {
            let messages = prepared(conversation(total));
            let elided = total - sent(&messages).len();
            assert!(elided >= elided_before, "the cut went back at {total}");
            if let Some(previous) = &previous {
                // The start of the request (all but its last two messages) matches the last one
                // unless the cut moved.
                if elided == elided_before {
                    assert_eq!(previous[..total - 2], messages[..total - 2], "prefix changed at {total}");
                }
            }
            elided_before = elided;
            previous = Some(messages);
        }
    }

    #[test]
    fn a_budget_of_bytes_applies_too() {
        let big = |index: usize| Image::new("image/jpeg", "x".repeat(3 * 1024 * 1024)).labelled(format!("big {index}"));
        let mut messages: Vec<Message> = (0..5).map(|index| turn(vec![result(&format!("t{index}"), [big(index)])])).collect();
        messages.push(turn(vec![result("small", [image("s")])]));
        let messages = prepared(messages);
        // 5 × 3 MB + a little is over 12 MB: the oldest go, in one batch (all five, as fewer than six).
        assert_eq!(sent(&messages), ["s"]);
    }

    #[test]
    fn a_result_with_several_pictures_is_elided_from_its_oldest() {
        let mut messages = vec![turn(vec![result("t", (0..MAX_IMAGES + 4).map(|index| image(&index.to_string())))])];
        messages.push(turn(vec![result("u", [image("last")])]));
        let messages = prepared(messages);
        assert_eq!(sent(&messages).first().map(String::as_str), Some("6"));
        let Block::ToolResult { content, .. } = &messages[0].content[0] else { panic!() };
        assert_eq!(content.lines().count(), 1 + BATCH);
    }

    #[test]
    fn the_users_pictures_count_and_become_a_line_when_elided() {
        let attached = Block::Image { media_type: "image/png".into(), data: "mine".into(), label: Some("design.png".into()) };
        let mut messages = vec![turn(vec![Block::Text { text: "look".into() }, attached.clone()])];
        messages.extend(conversation(MAX_IMAGES));
        let messages = prepared(messages);
        assert_eq!(
            messages[0].content[1],
            Block::Text { text: "[A picture the user attached (design.png) is elided from this request: you already viewed it.]".into() }
        );
        // Their newest message's pictures are always sent.
        let mut fresh = conversation(MAX_IMAGES);
        fresh.push(turn(vec![attached.clone(); 3]));
        let fresh = prepared(fresh);
        assert_eq!(fresh.last().unwrap().content, vec![attached; 3]);
    }

    #[test]
    fn a_model_without_images_gets_a_note_instead() {
        let mut messages = conversation(2);
        messages[1].content.push(Block::Text { text: "and this".into() });
        let Block::ToolResult { images, .. } = &mut messages[1].content[0] else { panic!() };
        images.push(image("extra"));
        messages.push(turn(vec![Block::Image { media_type: "image/png".into(), data: "mine".into(), label: None }]));
        prepare(&mut messages, false);
        assert!(sent(&messages).is_empty());
        let Block::ToolResult { content, .. } = &messages[0].content[0] else { panic!() };
        assert_eq!(content, "result t0\n[1 image left out: this model does not accept images.]");
        let Block::ToolResult { content, .. } = &messages[1].content[0] else { panic!() };
        assert_eq!(content, "result t1\n[2 images left out: this model does not accept images.]");
        assert_eq!(messages[1].content.len(), 2, "blocks stay where they were");
        assert_eq!(messages[2].content, vec![Block::Text { text: "[A picture the user attached is left out: this model does not accept images.]".into() }]);
    }

    #[test]
    fn one_turn_keeps_at_most_the_limit_and_says_what_it_cut() {
        let mut results = vec![
            result("a", (0..15).map(|index| image(&format!("a{index}")))),
            result("b", (0..15).map(|index| image(&format!("b{index}")))),
        ];
        cap_turn(&mut results);
        let Block::ToolResult { images, content, .. } = &results[0] else { panic!() };
        assert_eq!((images.len(), content.as_str()), (15, "result a"));
        let Block::ToolResult { images, content, .. } = &results[1] else { panic!() };
        assert_eq!(images.len(), 5);
        assert_eq!(content, "result b\n[10 more pictures were not kept: one step can return at most 20 pictures. Ask for fewer at a time.]");
    }

    #[test]
    fn one_picture_is_kept_however_large() {
        let mut results = vec![result("a", [Image::new("image/png", "x".repeat(MAX_BYTES + 1))])];
        cap_turn(&mut results);
        let Block::ToolResult { images, .. } = &results[0] else { panic!() };
        assert_eq!(images.len(), 1);
    }
}
