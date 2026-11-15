//! A plan written as prose instead of submitted with `plan_mode_complete`.
//!
//! Weaker models sometimes answer with a `<proposed_plan>` block. A single well-formed block
//! counts as a submission; anything else that looks like an attempt is reported, so the user
//! knows why no plan appeared. Ported from version 2 (from `@narumitw/pi-plan-mode` v0.58.3, MIT).

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposed {
    Absent,
    Valid(String),
    Empty,
    Multiple,
    Malformed,
    Unclosed,
}

const OPEN: &str = "<proposed_plan>";
const CLOSE: &str = "</proposed_plan>";

fn count(text: &str, tag: &str) -> usize {
    text.to_ascii_lowercase().matches(tag).count()
}

/// Reads a reply for a proposed plan. The tags must each be alone on a line.
pub fn parse(text: &str) -> Proposed {
    let (opening, closing) = (count(text, OPEN), count(text, CLOSE));
    match (opening, closing) {
        (0, 0) => return Proposed::Absent,
        (opening, closing) if opening > 1 || closing > 1 => return Proposed::Multiple,
        (1, 0) => return Proposed::Unclosed,
        (1, 1) => {}
        _ => return Proposed::Malformed,
    }
    let lines: Vec<&str> = text.lines().collect();
    let alone = |line: &str, tag: &str| line.strip_prefix(tag).is_some_and(|rest| rest.chars().all(|character| character == ' ' || character == '\t'));
    let (Some(start), Some(end)) = (lines.iter().position(|line| alone(line, OPEN)), lines.iter().position(|line| alone(line, CLOSE))) else {
        return Proposed::Malformed;
    };
    if end <= start {
        return Proposed::Malformed;
    }
    let plan = lines[start + 1..end].join("\n").trim().to_string();
    if plan.is_empty() { Proposed::Empty } else { Proposed::Valid(plan) }
}

/// What the user is told when a reply tried to propose a plan and failed.
pub fn invalid_message(proposed: &Proposed) -> Option<String> {
    let detail = match proposed {
        Proposed::Empty => "the block is empty",
        Proposed::Multiple => "more than one plan block was produced",
        Proposed::Malformed => "the tags must be on their own lines",
        Proposed::Unclosed => "the closing tag is missing",
        Proposed::Absent | Proposed::Valid(_) => return None,
    };
    Some(format!("The proposed plan is not ready: {detail}. The agent continues planning and must submit the plan with plan_mode_complete."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_well_formed_block_is_a_plan() {
        assert_eq!(parse("Here it is.\n<proposed_plan>\n# Title\n\n1. Do it\n</proposed_plan>\nThanks."), Proposed::Valid("# Title\n\n1. Do it".into()));
        assert_eq!(parse("<proposed_plan>  \r\nPlan\r\n</proposed_plan>\t"), Proposed::Valid("Plan".into()));
        assert_eq!(parse("Just an answer."), Proposed::Absent);
    }

    #[test]
    fn anything_else_is_named() {
        assert_eq!(parse("<proposed_plan>\n\n</proposed_plan>"), Proposed::Empty);
        assert_eq!(parse("<proposed_plan>\na\n</proposed_plan>\n<proposed_plan>\nb\n</proposed_plan>"), Proposed::Multiple);
        assert_eq!(parse("<proposed_plan>\nnever closed"), Proposed::Unclosed);
        assert_eq!(parse("The plan: <proposed_plan>inline</proposed_plan>"), Proposed::Malformed);
        assert_eq!(parse("</proposed_plan>"), Proposed::Malformed);
        assert!(invalid_message(&Proposed::Unclosed).unwrap().contains("the closing tag is missing"));
        assert_eq!(invalid_message(&Proposed::Absent), None);
    }
}
