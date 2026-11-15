//! `ask_user_question`: a structured question the model can put to the user mid-task.
//!
//! Ported from version 2 (itself adapted from `@narumitw/pi-plan-mode` v0.58.3, MIT). The schema
//! and every text the model reads are version 2's, word for word.
//!
//! - Every outcome is an ordinary result, never an error: an invalid call, nobody to ask, and a
//!   dismissed card each tell the model what to do instead.
//! - In Ultra Plan the card also offers "Write the plan now". Once the user presses it, every
//!   later call in the same run gets the wrap-up instruction back without a card, so an eager
//!   model cannot keep interviewing. A run's tools are built fresh, so the next message lifts it.
//! - The transcript marks the user's picks from the result text (`parse_answers`), so
//!   `format_answers` and `parse_answers` must stay each other's inverse.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Loading, Tool, ToolContext, ToolSpec};

pub const ASK_USER_QUESTION: &str = "ask_user_question";
/// The longest free-form answer the card accepts.
pub const MAX_ANSWER_LENGTH: usize = 4_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    /// A short label for the question's tab.
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// One question's answer. A free-form answer replaces the preset choices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestionAnswer {
    pub question_id: String,
    /// The labels of the chosen options.
    pub selected: Vec<String>,
    pub custom: Option<String>,
}

/// What came back from the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Answered(Vec<QuestionAnswer>),
    /// Dismissed without answering, or the run was stopped.
    Cancelled,
    /// Ultra Plan's "Write the plan now".
    WrapUp,
}

const CANCEL_ADVICE: &str = "Ask concisely in plain text or proceed only with a clearly stated low-risk assumption.";
const WRAP_UP_INSTRUCTION: &str = "The user wants to stop answering questions and review the plan now. Do not call ask_user_question again this turn. Submit the complete plan with plan_mode_complete, resolving every open decision (including any you just asked about) with your recommended answer and listing those as explicit assumptions.";

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["questions"],
        "properties": {
            "questions": {
                "type": "array",
                "minItems": 1,
                "maxItems": 3,
                "description": "Questions to ask the user. Prefer 1 and do not exceed 3.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "header", "question", "options"],
                    "properties": {
                        "id": { "type": "string", "description": "Stable snake_case identifier for mapping answers." },
                        "header": { "type": "string", "description": "Short tab label shown in the UI (12 or fewer characters)." },
                        "question": { "type": "string", "description": "Single-sentence question shown to the user." },
                        "multiSelect": { "type": "boolean", "description": "Allow selecting several options instead of exactly one. Default false." },
                        "options": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 4,
                            "description": "Provide 2-4 mutually exclusive choices. Put the recommended option first when there is a clear default. The user can always pick a free-form Other answer instead.",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["label", "description"],
                                "properties": {
                                    "label": { "type": "string", "description": "User-facing label (1-5 words)." },
                                    "description": { "type": "string", "description": "One short sentence explaining impact/tradeoff if selected." }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

fn text_field(value: &Value) -> String {
    value.as_str().map(str::trim).unwrap_or_default().to_string()
}

/// Checks a call's input and reads its questions. The error is a phrase the model is shown.
pub fn normalize(input: &Value) -> Result<Vec<Question>, String> {
    let Some(raw) = input.get("questions").and_then(Value::as_array) else {
        return Err("questions must be an array".to_string());
    };
    if raw.is_empty() || raw.len() > 3 {
        return Err("questions must contain 1-3 items".to_string());
    }
    let mut questions = Vec::with_capacity(raw.len());
    for (index, entry) in raw.iter().enumerate() {
        let number = index + 1;
        if !entry.is_object() {
            return Err(format!("question {number} must be an object"));
        }
        let (id, header, question) = (text_field(&entry["id"]), text_field(&entry["header"]), text_field(&entry["question"]));
        if id.is_empty() || header.is_empty() || question.is_empty() {
            return Err(format!("question {number} requires non-empty id, header, and question"));
        }
        let Some(raw_options) = entry["options"].as_array() else {
            return Err(format!("question {number} options must be an array"));
        };
        if raw_options.len() < 2 || raw_options.len() > 4 {
            return Err(format!("question {number} options must contain 2-4 items"));
        }
        let mut options = Vec::with_capacity(raw_options.len());
        for (option_index, option) in raw_options.iter().enumerate() {
            let option_number = option_index + 1;
            if !option.is_object() {
                return Err(format!("question {number} option {option_number} must be an object"));
            }
            let (label, description) = (text_field(&option["label"]), text_field(&option["description"]));
            if label.is_empty() || description.is_empty() {
                return Err(format!("question {number} option {option_number} requires a label and a description"));
            }
            options.push(QuestionOption { label, description });
        }
        questions.push(Question { id, header, question, options, multi_select: entry["multiSelect"] == true });
    }
    Ok(questions)
}

/// The result the model reads once the user has answered.
pub fn format_answers(questions: &[Question], answers: &[QuestionAnswer]) -> String {
    let mut lines = vec!["The user answered your questions:".to_string()];
    for question in questions {
        let Some(answer) = answers.iter().find(|answer| answer.question_id == question.id) else {
            continue;
        };
        lines.push(String::new());
        lines.push(format!("**{}** — {}", question.header, question.question));
        lines.extend(answer.selected.iter().map(|selected| format!("- {selected}")));
        if let Some(custom) = &answer.custom {
            lines.push(format!("- Other: {custom}"));
        }
    }
    lines.join("\n")
}

/// Reads a result written by [`format_answers`] back into answers, for the transcript's record of
/// what was asked. `None` when the result is not an answer (cancelled, wrapped up).
pub fn parse_answers(questions: &[Question], result: &str) -> Option<Vec<QuestionAnswer>> {
    let mut lines = result.lines();
    if lines.next()? != "The user answered your questions:" {
        return None;
    }
    let mut answers: Vec<QuestionAnswer> = Vec::new();
    let mut current: Option<&Question> = None;
    for line in lines {
        if let Some(question) = questions.iter().find(|question| line == format!("**{}** — {}", question.header, question.question)) {
            answers.push(QuestionAnswer { question_id: question.id.clone(), ..QuestionAnswer::default() });
            current = Some(question);
            continue;
        }
        let (Some(question), Some(answer)) = (current, answers.last_mut()) else {
            continue;
        };
        if let Some(item) = line.strip_prefix("- ") {
            if question.options.iter().any(|option| option.label == item) {
                answer.selected.push(item.to_string());
            } else if let Some(custom) = item.strip_prefix("Other: ") {
                answer.custom = Some(custom.to_string());
            }
        } else if let Some(custom) = &mut answer.custom {
            // A free-form answer may run over several lines.
            custom.push('\n');
            custom.push_str(line);
        }
    }
    for answer in &mut answers {
        if let Some(custom) = &mut answer.custom {
            custom.truncate(custom.trim_end().len());
        }
    }
    Some(answers)
}

fn cancelled(message: &str) -> String {
    format!("{message} {CANCEL_ADVICE}")
}

pub struct AskUserQuestion {
    /// Whether Ultra Plan is on, which offers the wrap-up button.
    interviewing: Arc<dyn Fn() -> bool + Send + Sync>,
    wrap_up_requested: AtomicBool,
}

impl AskUserQuestion {
    pub fn new(interviewing: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self { interviewing, wrap_up_requested: AtomicBool::new(false) }
    }
}

#[async_trait]
impl Tool for AskUserQuestion {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: ASK_USER_QUESTION.into(),
            description: "Ask the user 1-3 structured questions with meaningful preset options when a preference, tradeoff, or missing detail materially changes what you would do. Do not ask about facts you can discover from the repository or system.".into(),
            input_schema: schema(),
            loading: Loading::Always,
            // One card at a time.
            parallel_safe: false,
            hint: "ask the user structured multiple-choice questions when a decision is ambiguous".into(),
            guidelines: vec![
                "Ask about decisions the code cannot resolve, with 2–4 options and your recommendation first. Never ask whether an agreed plan is OK.".into(),
                "Use ask_user_question for important choices the codebase cannot answer: approach, scope, UX preferences. Each question needs 2-4 mutually exclusive options; the user can always answer with free-form text instead.".into(),
                "Do not use ask_user_question for facts discoverable from the repository, trivial decisions, or confirmations of an obvious next step.".into(),
            ],
            ends_run: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<String, String> {
        let questions = match normalize(&input) {
            Ok(questions) => questions,
            Err(error) => return Ok(cancelled(&format!("Error: {error}."))),
        };
        let offer_wrap_up = (self.interviewing)();
        if offer_wrap_up && self.wrap_up_requested.load(Ordering::SeqCst) {
            return Ok(WRAP_UP_INSTRUCTION.to_string());
        }
        if !context.user.present() {
            return Ok(cancelled("Interactive UI is not available, so the questions could not be shown."));
        }
        Ok(match context.user.ask(questions.clone(), offer_wrap_up).await {
            Answer::WrapUp => {
                self.wrap_up_requested.store(true, Ordering::SeqCst);
                WRAP_UP_INSTRUCTION.to_string()
            }
            Answer::Cancelled => cancelled("The user dismissed the questions without answering."),
            Answer::Answered(answers) => format_answers(&questions, &answers),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        json!({ "questions": [
            {
                "id": "scope", "header": " Scope ", "question": "How far should this go?",
                "options": [
                    { "label": "Just the bug", "description": "Smallest change." },
                    { "label": "Bug and tests", "description": "Adds coverage." }
                ]
            },
            {
                "id": "targets", "header": "Targets", "question": "Which platforms?", "multiSelect": true,
                "options": [
                    { "label": "macOS", "description": "The app." },
                    { "label": "CLI", "description": "The command." },
                    { "label": "Both later", "description": "Decide after." }
                ]
            }
        ] })
    }

    #[test]
    fn questions_are_trimmed_and_checked() {
        let questions = normalize(&input()).unwrap();
        assert_eq!(questions[0].header, "Scope");
        assert!(!questions[0].multi_select);
        assert!(questions[1].multi_select);
        assert_eq!(questions[1].options.len(), 3);

        assert_eq!(normalize(&json!({})).unwrap_err(), "questions must be an array");
        assert_eq!(normalize(&json!({ "questions": [] })).unwrap_err(), "questions must contain 1-3 items");
        let four = json!({ "questions": [input()["questions"][0], input()["questions"][0], input()["questions"][0], input()["questions"][0]] });
        assert_eq!(normalize(&four).unwrap_err(), "questions must contain 1-3 items");
        let mut one_option = input();
        one_option["questions"][0]["options"].as_array_mut().unwrap().pop();
        assert_eq!(normalize(&one_option).unwrap_err(), "question 1 options must contain 2-4 items");
        let mut blank = input();
        blank["questions"][1]["options"][2]["description"] = json!("  ");
        assert_eq!(normalize(&blank).unwrap_err(), "question 2 option 3 requires a label and a description");
        let mut no_header = input();
        no_header["questions"][0]["header"] = json!("");
        assert_eq!(normalize(&no_header).unwrap_err(), "question 1 requires non-empty id, header, and question");
    }

    #[test]
    fn answers_read_back_as_they_were_written() {
        let questions = normalize(&input()).unwrap();
        let answers = vec![
            QuestionAnswer { question_id: "scope".into(), selected: vec![], custom: Some("Only the parser,\nnot the lexer".into()) },
            QuestionAnswer { question_id: "targets".into(), selected: vec!["macOS".into(), "CLI".into()], custom: None },
        ];
        let text = format_answers(&questions, &answers);
        assert_eq!(
            text,
            "The user answered your questions:\n\n**Scope** — How far should this go?\n- Other: Only the parser,\nnot the lexer\n\n**Targets** — Which platforms?\n- macOS\n- CLI"
        );
        let parsed = parse_answers(&questions, &text).unwrap();
        assert_eq!(parsed, answers);
        assert_eq!(parse_answers(&questions, WRAP_UP_INSTRUCTION), None);
    }

    #[tokio::test]
    async fn with_nobody_to_ask_the_model_is_told_to_carry_on() {
        let tool = AskUserQuestion::new(Arc::new(|| false));
        let context = ToolContext::detached("/tmp");
        let result = tool.run(input(), &context).await.unwrap();
        assert!(result.starts_with("Interactive UI is not available"));
        let result = tool.run(json!({ "questions": "x" }), &context).await.unwrap();
        assert_eq!(result, format!("Error: questions must be an array. {CANCEL_ADVICE}"));
    }
}
