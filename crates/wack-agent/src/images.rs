//! What becomes of tool results' pictures on their way to the model.
//!
//! The session keeps every picture a tool returned, as it arrived; this runs on each request's
//! copy of the messages and never on the session. Two rules:
//! - A model that does not accept images gets none: each result's pictures are dropped and its
//!   text says so. Decided per request, since a chat can carry on with another model.
//! - Otherwise only the newest pictures are sent. Older ones become `[screenshot elided]`, in
//!   batches: the cut is a function of how many pictures there are and moves only every
//!   `BATCH` of them, so between moves the start of the request is unchanged and the provider's
//!   prompt cache holds. Between [`KEEP`] and `KEEP + BATCH - 1` pictures are sent.
//!
//! Pictures the user attached (`Block::Image`) are the user's words: never counted or elided.
//! Both rules keep every message and block where it was, so a tool call and its result stay
//! paired.

use wack_llm::{Block, Image, Message};

/// The newest pictures always sent.
pub const KEEP: usize = 3;
/// How many older pictures go at once when the cut moves.
const BATCH: usize = 6;
pub const ELIDED: &str = "[screenshot elided]";

/// Applies both rules to a request's messages.
pub fn prepare(messages: &mut [Message], accepts_images: bool) {
    if !accepts_images {
        for (content, images) in results(messages) {
            let count = images.len();
            images.clear();
            let noun = if count == 1 { "image" } else { "images" };
            append(content, &format!("[{count} {noun} left out: this model does not accept images.]"));
        }
        return;
    }
    let total: usize = results(messages).map(|(_, images)| images.len()).sum();
    let mut elide = elided_count(total);
    for (content, images) in results(messages) {
        if elide == 0 {
            break;
        }
        let here = elide.min(images.len());
        images.drain(..here);
        for _ in 0..here {
            append(content, ELIDED);
        }
        elide -= here;
    }
}

/// Each tool result that has pictures, oldest first, as its text and its pictures.
fn results(messages: &mut [Message]) -> impl Iterator<Item = (&mut String, &mut Vec<Image>)> {
    messages.iter_mut().flat_map(|message| message.content.iter_mut()).filter_map(|block| match block {
        Block::ToolResult { content, images, .. } if !images.is_empty() => Some((content, images)),
        _ => None,
    })
}

/// How many of the oldest pictures to elide when there are `total`.
fn elided_count(total: usize) -> usize {
    if total <= KEEP { 0 } else { (total - KEEP) / BATCH * BATCH }
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
        Image { media_type: "image/png".into(), data: data.into() }
    }

    /// One tool-result message per picture, numbered from 0, oldest first.
    fn conversation(pictures: usize) -> Vec<Message> {
        (0..pictures)
            .map(|index| Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    tool_use_id: format!("t{index}"),
                    content: format!("shot {index}"),
                    is_error: false,
                    images: vec![image(&index.to_string())],
                }],
            })
            .collect()
    }

    fn sent(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|message| &message.content)
            .flat_map(|block| match block {
                Block::ToolResult { images, .. } => images.iter().map(|image| image.data.clone()).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn the_cut_moves_in_batches_so_the_cache_holds_between_moves() {
        for total in 0..=8 {
            let mut messages = conversation(total);
            prepare(&mut messages, true);
            assert_eq!(sent(&messages).len(), total, "nothing is elided at {total}");
        }
        // From the ninth picture the oldest six go, and stay the same six until the fifteenth.
        for total in 9..=14 {
            let mut messages = conversation(total);
            prepare(&mut messages, true);
            assert_eq!(sent(&messages).first().map(String::as_str), Some("6"), "at {total}");
            assert_eq!(sent(&messages).len(), total - 6);
            let Block::ToolResult { content, .. } = &messages[5].content[0] else { panic!() };
            assert_eq!(content, "shot 5\n[screenshot elided]");
        }
        let mut messages = conversation(15);
        prepare(&mut messages, true);
        assert_eq!(sent(&messages), ["12", "13", "14"]);
    }

    #[test]
    fn the_prefix_is_identical_until_the_cut_moves() {
        let mut nine = conversation(9);
        let mut fourteen = conversation(14);
        prepare(&mut nine, true);
        prepare(&mut fourteen, true);
        assert_eq!(nine[..], fourteen[..9]);
    }

    #[test]
    fn a_result_with_several_pictures_is_elided_from_its_oldest() {
        let mut messages = vec![Message {
            role: Role::User,
            content: vec![Block::ToolResult {
                tool_use_id: "t".into(),
                content: String::new(),
                is_error: false,
                images: (0..10).map(|index| image(&index.to_string())).collect(),
            }],
        }];
        prepare(&mut messages, true);
        assert_eq!(sent(&messages), ["6", "7", "8", "9"]);
        let Block::ToolResult { content, .. } = &messages[0].content[0] else { panic!() };
        assert_eq!(content.lines().count(), 6);
        assert!(content.lines().all(|line| line == ELIDED));
    }

    #[test]
    fn a_model_without_images_gets_a_note_instead() {
        let mut messages = conversation(2);
        messages[1].content.push(Block::Text { text: "and this".into() });
        let Block::ToolResult { images, .. } = &mut messages[1].content[0] else { panic!() };
        images.push(image("extra"));
        prepare(&mut messages, false);
        assert!(sent(&messages).is_empty());
        let Block::ToolResult { content, .. } = &messages[0].content[0] else { panic!() };
        assert_eq!(content, "shot 0\n[1 image left out: this model does not accept images.]");
        let Block::ToolResult { content, .. } = &messages[1].content[0] else { panic!() };
        assert_eq!(content, "shot 1\n[2 images left out: this model does not accept images.]");
        assert_eq!(messages[1].content.len(), 2, "blocks stay where they were");
    }

    #[test]
    fn the_users_own_pictures_are_never_counted_or_elided() {
        let mut messages = conversation(8);
        messages.insert(
            0,
            Message {
                role: Role::User,
                content: vec![Block::Image { media_type: "image/png".into(), data: "mine".into() }; 5],
            },
        );
        let before = messages.clone();
        prepare(&mut messages, true);
        assert_eq!(messages, before);
        prepare(&mut messages, false);
        assert_eq!(messages[0], before[0]);
    }
}
