//! Element refs across calls. Every walk that returns refs (`tree`, `find`) starts a new
//! generation `e{n}`; refs from an older generation fail with "call `tree` again", the way
//! `computer_act` treats a stale stateId. Handles (`AXUIElement`s) stay on the helper's single
//! thread; nothing here is `Send` and it never needs to be.

use crate::ax::{self, Element};
use crate::outline;

pub struct Refs {
    generation: u64,
    /// The element each `e{generation}-{i}` resolves to. Candidates that name-matching reports
    /// in an error are appended too, so the agent can act on them directly.
    elements: Vec<Element>,
}

impl Default for Refs {
    fn default() -> Self {
        Self { generation: 0, elements: Vec::new() }
    }
}

impl Refs {
    /// Starts a generation and returns its ref prefix (`e3`).
    pub fn begin(&mut self) -> String {
        self.generation += 1;
        self.elements.clear();
        format!("e{}", self.generation)
    }

    /// `refs[i]` becomes `e{generation}-{i}`. For `tree`, the outline's handle list.
    pub fn register(&mut self, elements: Vec<Element>) {
        self.elements = elements;
    }

    /// The next ref index (`find` numbers its matches by hand).
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Appends one element and returns the ref just assigned.
    pub fn push(&mut self, element: Element) -> String {
        let reference = format!("e{}-{}", self.generation, self.elements.len());
        self.elements.push(element);
        reference
    }

    /// Adds candidates to the current generation and returns their refs, for error text like
    /// "use a ref: e4-2, e7-9". No-op without a generation.
    pub fn offer(&mut self, elements: Vec<Element>) -> Vec<String> {
        let prefix = format!("e{}", self.generation);
        elements
            .into_iter()
            .map(|element| {
                let reference = format!("{prefix}-{}", self.elements.len());
                self.elements.push(element);
                reference
            })
            .collect()
    }

    /// The element a ref names, or the sentence explaining which recovery applies.
    pub fn resolve(&self, reference: &str) -> Result<Element, String> {
        let reference = reference.trim();
        let prefix = format!("e{}", self.generation);
        let index = outline::parse_ref(reference, &prefix).ok_or_else(|| {
            if reference.starts_with('e') && reference[1..].split('-').next().is_some_and(|digits| !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())) {
                format!("Ref {reference} is from an older tree; call `tree` again.")
            } else {
                format!("Ref {reference} isn't one of the latest refs (they look like {prefix}-0).")
            }
        })?;
        let element = self
            .elements
            .get(index)
            .ok_or_else(|| format!("Ref {reference} doesn't exist in the latest tree."))?;
        if !ax::is_alive(element) {
            return Err(format!("Ref {reference} is gone; call `tree` again."));
        }
        Ok(element.clone())
    }
}

#[cfg(test)]
mod tests {
    // The AX-free half: ref parsing and staleness. resolve() itself needs live elements, so its
    // generation logic is exercised through parse_ref, the same code path it uses.
    use crate::outline::parse_ref;

    #[test]
    fn refs_carry_their_generation() {
        assert_eq!(parse_ref("e3-12", "e3"), Some(12));
        assert_eq!(parse_ref("e2-12", "e3"), None);
        assert_eq!(parse_ref("e3-12", "e2"), None);
    }
}
