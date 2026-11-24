//! A small argument reader: named options with values, then whatever is left.

pub struct Args {
    items: Vec<String>,
}

impl Args {
    pub fn new(items: impl Iterator<Item = String>) -> Self {
        Self { items: items.collect() }
    }

    /// Takes `--name value` out of the arguments, wherever it is.
    pub fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(index) = self.items.iter().position(|item| item == name) else {
            return Ok(None);
        };
        if index + 1 >= self.items.len() {
            return Err(format!("{name} needs a value."));
        }
        self.items.remove(index);
        Ok(Some(self.items.remove(index)))
    }

    /// Takes `--name` (a switch, with no value) out of the arguments; true if it was there.
    pub fn flag(&mut self, name: &str) -> bool {
        let Some(index) = self.items.iter().position(|item| item == name) else { return false };
        self.items.remove(index);
        true
    }

    /// Takes every `--name value` out of the arguments.
    pub fn values(&mut self, name: &str) -> Result<Vec<String>, String> {
        let mut values = Vec::new();
        while let Some(value) = self.value(name)? {
            values.push(value);
        }
        Ok(values)
    }

    /// Takes the first remaining argument as the command.
    pub fn command(&mut self) -> Option<String> {
        (!self.items.is_empty()).then(|| self.items.remove(0))
    }

    /// Everything not taken yet.
    pub fn rest(&mut self) -> Vec<String> {
        std::mem::take(&mut self.items)
    }

    /// Fails if anything was not understood.
    pub fn finish(&mut self) -> Result<(), String> {
        match self.items.first() {
            Some(extra) => Err(format!("{extra} was not understood. Run wack help.")),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_can_sit_anywhere_and_the_rest_is_the_prompt() {
        let mut args = Args::new(["--data-dir", "/tmp/x", "run", "--model", "m1", "fix", "the", "--effort", "high", "bug"].map(String::from).into_iter());
        assert_eq!(args.value("--data-dir").unwrap().as_deref(), Some("/tmp/x"));
        assert_eq!(args.command().as_deref(), Some("run"));
        assert_eq!(args.value("--model").unwrap().as_deref(), Some("m1"));
        assert_eq!(args.value("--effort").unwrap().as_deref(), Some("high"));
        assert_eq!(args.value("--cwd").unwrap(), None);
        assert_eq!(args.rest(), ["fix", "the", "bug"]);
        assert!(args.finish().is_ok());
    }

    #[test]
    fn a_missing_value_and_leftovers_are_errors() {
        let mut args = Args::new(["connect", "--model", "a", "--model", "b", "--name"].map(String::from).into_iter());
        args.command();
        assert_eq!(args.values("--model").unwrap(), ["a", "b"]);
        assert!(args.value("--name").is_err());
        assert!(args.finish().is_err());
    }
}
