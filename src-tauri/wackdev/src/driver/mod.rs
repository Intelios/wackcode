//! The dev-app driver's own rules, layered on the shared platform modules. `target` is the one
//! safety rule that matters: the driver only ever aims at the dev build of this repository's
//! own checkout. `refs` keeps the element handles an outline assigns valid until the next
//! walk — both are pure, so the rules are unit-tested without a running app.

pub mod refs;
pub mod target;
