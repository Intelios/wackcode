//! `computer_clipboard`: reading what the user copied, or putting text on the clipboard.
//!
//! Invariants:
//! - Reading and writing are separate grants, each asked for on its own card the first time.
//! - An item a password manager marks concealed or transient is never read
//!   (`native::clipboard`); what is read is fenced as app data and cut at [`MAX_READ`] characters.

use serde::Deserialize;
use serde_json::Value;
use wack_agent::ToolOutput;

use super::{Call, app_data};

pub const MAX_READ: usize = 20_000;
pub const MAX_WRITE: usize = 100_000;

#[derive(Deserialize)]
struct ClipboardInput {
    action: String,
    #[serde(default)]
    text: Option<String>,
}

pub(super) async fn clipboard(call: &Call<'_>, input: Value) -> Result<ToolOutput, String> {
    let input: ClipboardInput = serde_json::from_value(input).map_err(|error| format!("computer_clipboard didn't understand its input: {error}."))?;
    match input.action.as_str() {
        "read" => {
            call.chat.clipboard_access(false, call.present, &call.context.cancel).await?;
            match call.chat.desktop().clipboard_read().await? {
                None => Ok("The clipboard holds no text.".to_string().into()),
                Some(text) => {
                    let total = text.chars().count();
                    let shown: String = text.chars().take(MAX_READ).collect();
                    let mut answer = format!("The clipboard holds {total} characters:\n{}", app_data(&shown));
                    if total > MAX_READ {
                        answer.push_str(&format!("\nOnly the first {MAX_READ} are shown."));
                    }
                    Ok(answer.into())
                }
            }
        }
        "write" => {
            let text = input.text.filter(|text| !text.is_empty()).ok_or("write needs `text`.")?;
            if text.chars().count() > MAX_WRITE {
                return Err(format!("That is more than the {MAX_WRITE} characters computer_clipboard copies."));
            }
            call.chat.clipboard_access(true, call.present, &call.context.cancel).await?;
            call.chat.desktop().clipboard_write(&text).await?;
            Ok(format!("Copied {} characters to the clipboard, replacing what was there.", text.chars().count()).into())
        }
        other => Err(format!("“{other}” isn't an action: use read or write.")),
    }
}
