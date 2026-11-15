//! Answering the agent's questions on the terminal.
//!
//! A question prints with numbered options, one question at a time, and the next line typed is
//! its answer: a number (`2`), several for a multi-select (`1,3`), `w` to stop an Ultra Plan
//! interview and have the plan written, an empty line to dismiss the questions, or any other
//! text as a free-form answer. While a question waits, typed lines answer it rather than being
//! sent to the agent as messages.

use std::sync::Mutex;

use wack_agent::{
    Control,
    ask::{Answer, Question, QuestionAnswer},
};

use crate::print::{dim, paint};

struct Waiting {
    id: String,
    questions: Vec<Question>,
    wrap_up: bool,
    answers: Vec<QuestionAnswer>,
}

/// The question on screen, if any.
#[derive(Default)]
pub struct Asking {
    waiting: Mutex<Option<Waiting>>,
}

/// What one typed line means for one question.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Answer(QuestionAnswer),
    WrapUp,
    Cancel,
}

pub fn read_reply(question: &Question, line: &str, wrap_up: bool) -> Reply {
    let line = line.trim();
    if line.is_empty() {
        return Reply::Cancel;
    }
    if wrap_up && line.eq_ignore_ascii_case("w") {
        return Reply::WrapUp;
    }
    let numbers: Option<Vec<usize>> = line.split(',').map(|part| part.trim().parse::<usize>().ok()).collect();
    let picked = numbers.filter(|numbers| {
        numbers.iter().all(|number| (1..=question.options.len()).contains(number)) && (question.multi_select || numbers.len() == 1)
    });
    let answer = match picked {
        Some(numbers) => {
            let mut selected: Vec<String> = Vec::new();
            for number in numbers {
                let label = question.options[number - 1].label.clone();
                if !selected.contains(&label) {
                    selected.push(label);
                }
            }
            QuestionAnswer { question_id: question.id.clone(), selected, custom: None }
        }
        None => QuestionAnswer { question_id: question.id.clone(), selected: Vec::new(), custom: Some(line.to_string()) },
    };
    Reply::Answer(answer)
}

fn show(question: &Question, number: usize, total: usize, wrap_up: bool) {
    let count = if total > 1 { format!(" ({number} of {total})") } else { String::new() };
    println!("\n{} {}", paint("1;36", &format!("? {}{count}", question.header)), question.question);
    for (index, option) in question.options.iter().enumerate() {
        println!("  {} {} {}", paint("1", &format!("{}.", index + 1)), option.label, dim(&format!("— {}", option.description)));
    }
    let pick = if question.multi_select { "numbers separated by commas" } else { "a number" };
    let stop = if wrap_up { ", w to have the plan written now" } else { "" };
    println!("{}", dim(&format!("Type {pick}, or your own answer{stop}. An empty line dismisses the question.")));
}

impl Asking {
    /// Puts a question on screen. Its answer is the next line typed.
    pub fn ask(&self, id: String, questions: Vec<Question>, wrap_up: bool) {
        if let Some(first) = questions.first() {
            show(first, 1, questions.len(), wrap_up);
        }
        *self.waiting.lock().unwrap() = Some(Waiting { id, questions, wrap_up, answers: Vec::new() });
    }

    /// Takes a typed line as the answer to the question on screen. False when there is none, so
    /// the line is a message for the agent instead.
    pub fn answer(&self, line: &str, control: &Control) -> bool {
        let mut slot = self.waiting.lock().unwrap();
        let Some(waiting) = slot.as_mut() else {
            return false;
        };
        let question = &waiting.questions[waiting.answers.len()];
        let done = match read_reply(question, line, waiting.wrap_up) {
            Reply::Cancel => Some(Answer::Cancelled),
            Reply::WrapUp => Some(Answer::WrapUp),
            Reply::Answer(answer) => {
                waiting.answers.push(answer);
                match waiting.questions.get(waiting.answers.len()) {
                    Some(next) => {
                        show(next, waiting.answers.len() + 1, waiting.questions.len(), waiting.wrap_up);
                        None
                    }
                    None => Some(Answer::Answered(std::mem::take(&mut waiting.answers))),
                }
            }
        };
        if let Some(answer) = done {
            let id = waiting.id.clone();
            *slot = None;
            control.answer(&id, answer);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use wack_agent::ask::QuestionOption;

    use super::*;

    fn question(multi_select: bool) -> Question {
        let option = |label: &str| QuestionOption { label: label.into(), description: "Why.".into() };
        Question {
            id: "scope".into(),
            header: "Scope".into(),
            question: "How far?".into(),
            options: vec![option("Small"), option("Large"), option("Both")],
            multi_select,
        }
    }

    fn picked(reply: Reply) -> (Vec<String>, Option<String>) {
        match reply {
            Reply::Answer(answer) => (answer.selected, answer.custom),
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[test]
    fn a_line_is_a_pick_a_free_answer_or_a_way_out() {
        let single = question(false);
        assert_eq!(picked(read_reply(&single, " 2 ", false)), (vec!["Large".to_string()], None));
        assert_eq!(picked(read_reply(&single, "only the parser", false)), (vec![], Some("only the parser".to_string())));
        // Out of range, or several for a single choice, is taken as the user's own words.
        assert_eq!(picked(read_reply(&single, "7", false)), (vec![], Some("7".to_string())));
        assert_eq!(picked(read_reply(&single, "1,2", false)), (vec![], Some("1,2".to_string())));
        assert_eq!(read_reply(&single, "  ", false), Reply::Cancel);

        let multi = question(true);
        assert_eq!(picked(read_reply(&multi, "3, 1,3", false)), (vec!["Both".to_string(), "Small".to_string()], None));

        // `w` wraps up only where that is on offer.
        assert_eq!(read_reply(&single, "W", true), Reply::WrapUp);
        assert_eq!(picked(read_reply(&single, "w", false)), (vec![], Some("w".to_string())));
    }
}
