//! Plan mode's fail-closed shell policy: which `bash` commands may run while planning.
//!
//! Ported from version 2's `policy.ts` (itself from `@narumitw/pi-plan-mode` v0.58.3, MIT), with
//! its tests. A command runs only if every segment of it is recognised as reading: anything the
//! parser cannot follow (redirection, substitution, a newline, a background job) is refused
//! rather than guessed at. The `gh` read paths are always on; version 2's configurable extra git
//! subcommands had no switch in the app and are not ported.
//!
//! This is a policy, not a sandbox: allowed commands like `npm test` can still write caches or
//! run project hooks. It keeps the planning agent on the read-only side of the codebase; it does
//! not contain hostile input.

use std::path::Path;

const MUTATING_COMMANDS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "mkdir", "touch", "chmod", "chown", "chgrp", "ln", "tee", "truncate", "dd", "sudo", "su",
    "kill", "pkill", "killall", "reboot", "shutdown", "vim", "vi", "nano", "emacs", "code", "subl",
];

const READ_ONLY_COMMANDS: &[&str] = &[
    "cat", "head", "tail", "grep", "find", "ls", "pwd", "echo", "printf", "wc", "sort", "uniq", "diff", "file", "stat",
    "du", "df", "tree", "which", "whereis", "type", "printenv", "uname", "whoami", "id", "date", "uptime", "ps", "jq",
    "rg", "fd", "bat", "eza",
];

const GH_READ_PATHS: [&str; 4] = ["pr view", "pr list", "issue view", "issue list"];

/// The first part of `command` that may not run while planning, or `None` when all of it may.
pub fn blocked_segment(command: &str, cwd: Option<&Path>) -> Option<String> {
    match split_segments(command) {
        Some(segments) if !segments.is_empty() => segments.into_iter().find(|segment| !is_safe_segment(segment, cwd)),
        _ => {
            let trimmed = command.trim();
            Some(if trimmed.is_empty() { "(empty command)".to_string() } else { trimmed.to_string() })
        }
    }
}

pub fn is_safe_command(command: &str, cwd: Option<&Path>) -> bool {
    blocked_segment(command, cwd).is_none()
}

/// Splits a command at `;`, `|`, `||` and `&&`. `None` when it uses anything this policy does
/// not follow: redirection, grouping, a background job, a backtick or a line break.
fn split_segments(command: &str) -> Option<Vec<String>> {
    let trimmed = command.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r', '`']) {
        return None;
    }
    let characters: Vec<char> = trimmed.chars().collect();
    let text = |from: usize, to: usize| characters[from..to].iter().collect::<String>().trim().to_string();
    let mut segments = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0;
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if escaped {
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(open) = quote {
            if character == open {
                quote = None;
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if matches!(character, '>' | '<' | '(' | ')') {
            return None;
        } else {
            let next = characters.get(index + 1).copied();
            if character == '&' && next != Some('&') {
                return None;
            }
            let separator = match character {
                ';' | '|' => {
                    if next == Some(character) { 2 } else { 1 }
                }
                '&' => 2,
                _ => 0,
            };
            if separator > 0 {
                let segment = text(start, index);
                if segment.is_empty() {
                    return None;
                }
                segments.push(segment);
                index += separator - 1;
                start = index + 1;
            }
        }
        index += 1;
    }
    if quote.is_some() || escaped {
        return None;
    }
    let last = text(start.min(characters.len()), characters.len());
    if last.is_empty() {
        return None;
    }
    segments.push(last);
    Some(segments)
}

fn is_safe_segment(segment: &str, cwd: Option<&Path>) -> bool {
    if has_shell_expansion(segment) || assigns_variable(segment) {
        return false;
    }
    let Some(tokens) = shell_words(segment) else {
        return false;
    };
    let Some(first) = tokens.first() else {
        return false;
    };
    let command = first.to_lowercase();
    if MUTATING_COMMANDS.contains(&command.as_str()) {
        return false;
    }
    let args: Vec<&str> = tokens[1..].iter().map(String::as_str).collect();
    if !has_safe_arguments(&command, &args) {
        return false;
    }
    if command == "hostname" {
        return args.is_empty();
    }
    if READ_ONLY_COMMANDS.contains(&command.as_str()) {
        return true;
    }
    is_safe_structured_command(&command, &args, cwd)
}

/// `NAME=value` at the start of a word: an environment assignment.
fn assigns_variable(segment: &str) -> bool {
    let characters: Vec<char> = segment.chars().collect();
    (0..characters.len()).any(|start| {
        if start > 0 && !characters[start - 1].is_whitespace() {
            return false;
        }
        if !(characters[start].is_ascii_alphabetic() || characters[start] == '_') {
            return false;
        }
        let mut end = start + 1;
        while end < characters.len() && (characters[end].is_ascii_alphanumeric() || characters[end] == '_') {
            end += 1;
        }
        characters.get(end) == Some(&'=')
    })
}

fn has_shell_expansion(segment: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in segment.chars() {
        if escaped {
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(open) = quote {
            if character == open {
                quote = None;
            } else if character == '$' && open == '"' {
                return true;
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if matches!(character, '$' | '*' | '?' | '[' | '{') {
            return true;
        }
    }
    false
}

fn shell_words(segment: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in segment.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if let Some(open) = quote {
            if character == open {
                quote = None;
            } else {
                word.push(character);
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    Some(words)
}

/// `flag` itself, or `flag=value`.
fn is_option(argument: &str, flag: &str) -> bool {
    argument == flag || argument.strip_prefix(flag).is_some_and(|rest| rest.starts_with('='))
}

fn has_safe_arguments(command: &str, args: &[&str]) -> bool {
    // Global only because every command that offers them mutates: `--fix` rewrites files,
    // `--write` writes output, `find -delete` unlinks, and no allow-listed read-only command
    // takes any of them. sed's in-place flags (`-i`, `--in-place`) are deliberately not here:
    // for grep, rg, fd, diff, sort, uniq and file a bare `-i` only toggles case-insensitivity.
    // They are scoped to `sed` below, which also catches combined forms like `-ni`.
    if args.iter().any(|argument| matches!(*argument, "--fix" | "--write" | "-delete" | "--delete")) {
        return false;
    }
    let any = |test: &dyn Fn(&str) -> bool| args.iter().any(|argument| test(argument));
    match command {
        "sed" => !any(&|argument| {
            is_option(argument, "--in-place")
                || (argument.starts_with('-') && !argument.starts_with("--") && argument.len() > 1 && argument[1..].contains('i'))
        }),
        "find" => !any(&|argument| {
            matches!(argument, "-exec" | "-execdir" | "-ok" | "-okdir" | "-fprint" | "-fprint0" | "-fprintf" | "-fls")
        }),
        "date" => !any(&|argument| argument == "-s" || argument.starts_with("--set")),
        "sort" | "tree" => {
            let writes = any(&|argument| {
                argument == "-o" || (argument.starts_with("-o") && !argument.starts_with("--")) || argument.starts_with("--output")
            });
            let elsewhere = command == "sort"
                && any(&|argument| {
                    argument == "-T"
                        || (argument.starts_with("-T") && argument.len() > 2)
                        || argument.starts_with("--temporary-directory")
                        || argument.starts_with("--compress-program")
                });
            !writes && !elsewhere
        }
        "diff" => !any(&|argument| is_option(argument, "--output")),
        "uniq" => args.iter().filter(|argument| !argument.starts_with('-')).count() <= 1,
        "fd" => !any(&|argument| ["-x", "-X", "--exec", "--exec-batch"].iter().any(|flag| is_option(argument, flag))),
        "rg" => !any(&|argument| is_option(argument, "--pre")),
        "bat" => !any(&|argument| is_option(argument, "--pager")),
        _ => true,
    }
}

fn is_safe_structured_command(command: &str, args: &[&str], cwd: Option<&Path>) -> bool {
    match command {
        "git" => is_safe_git_command(args, cwd),
        "gh" => is_safe_gh_command(args),
        "sed" => {
            // Only printing a line range: `sed -n 10,20p file`.
            let quiet = args.iter().any(|argument| {
                *argument == "-n" || (argument.starts_with('-') && !argument[1..].contains('-') && argument[1..].contains('n'))
            });
            let script = args.iter().find(|argument| !argument.starts_with('-'));
            quiet && script.is_some_and(|script| is_line_range_print(script))
        }
        "node" | "python" | "python3" | "tsc" | "biome" | "ruff" | "ty" => {
            if args.contains(&"--version") {
                return true;
            }
            if matches!(command, "python" | "python3") && args.first() == Some(&"-m") && args.get(1) == Some(&"pytest") {
                return true;
            }
            command == "tsc"
                && args.contains(&"--noEmit")
                && !args.iter().any(|argument| {
                    ["--incremental", "--tsBuildInfoFile", "--generateTrace"].iter().any(|flag| is_option(argument, flag))
                })
        }
        "npm" | "pnpm" => is_safe_package_manager_command(command, args, cwd),
        "pytest" | "vitest" | "jest" => true,
        "cargo" | "go" => {
            let subcommand = args.iter().find(|argument| !argument.starts_with('-')).map(|argument| argument.to_lowercase());
            matches!(subcommand.as_deref(), Some("test" | "check"))
        }
        _ => false,
    }
}

/// `12p` or `10,20p`.
fn is_line_range_print(script: &str) -> bool {
    let Some(range) = script.strip_suffix('p') else {
        return false;
    };
    let number = |text: &str| !text.is_empty() && text.chars().all(|character| character.is_ascii_digit());
    match range.split_once(',') {
        Some((from, to)) => number(from) && number(to),
        None => number(range),
    }
}

/// `test`, `test:unit`, `check`, `typecheck` or `lint`.
fn is_check_script(name: Option<&str>) -> bool {
    let Some(name) = name else {
        return false;
    };
    if matches!(name, "check" | "typecheck" | "lint" | "test") {
        return true;
    }
    let Some(rest) = name.strip_prefix("test:") else {
        return false;
    };
    let mut characters = rest.chars();
    characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && characters.all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | ':' | '.' | '-'))
}

/// Test scripts may be named `test:web` or `test:unit`, and monorepos select a package before the
/// command. Only known selectors are consumed, so a directory or filter is never mistaken for the
/// script. `pnpm exec` permits local check and test runners only, never arbitrary executables.
fn is_safe_package_manager_command(command: &str, args: &[&str], cwd: Option<&Path>) -> bool {
    let (selectors, flags): (&[&str], &[&str]) = if command == "pnpm" {
        (&["--dir", "-C", "--filter", "-F"], &["--silent", "-s", "--recursive", "-r", "--workspace-root", "-w"])
    } else {
        (&["--prefix", "--workspace", "-w"], &["--silent", "-s"])
    };
    let mut index = 0;
    while let Some(option) = args.get(index).filter(|argument| argument.starts_with('-')) {
        if selectors.contains(option) {
            match args.get(index + 1) {
                Some(value) if !value.starts_with('-') => index += 2,
                _ => return false,
            }
        } else if selectors.iter().any(|selector| {
            option.strip_prefix(selector).and_then(|rest| rest.strip_prefix('=')).is_some_and(|value| !value.is_empty())
        }) || flags.contains(option)
        {
            index += 1;
        } else {
            return false;
        }
    }
    let action = args.get(index).copied();
    let action_args = &args[(index + 1).min(args.len())..];
    if action == Some("run") {
        return is_check_script(action_args.first().copied());
    }
    if is_check_script(action) {
        return true;
    }
    if command == "pnpm" && action == Some("exec") {
        return match action_args.first() {
            Some(runner) if matches!(*runner, "vitest" | "jest" | "pytest" | "tsc" | "cargo" | "go") => {
                is_safe_structured_command(runner, &action_args[1..], cwd)
            }
            _ => false,
        };
    }
    if action == Some("audit") && action_args.contains(&"fix") {
        return false;
    }
    matches!(action, Some("list" | "ls" | "view" | "info" | "search" | "outdated" | "audit"))
}

fn is_safe_git_command(args: &[&str], cwd: Option<&Path>) -> bool {
    // Global options: only `--no-pager`, and `-C` when it names the working directory itself.
    let mut index = 0;
    while let Some(argument) = args.get(index) {
        if *argument == "--no-pager" {
            index += 1;
        } else if *argument == "-C" {
            match args.get(index + 1) {
                Some(directory) if is_working_directory(directory, cwd) => index += 2,
                _ => return false,
            }
        } else {
            break;
        }
    }
    let Some(subcommand) = args.get(index).map(|argument| argument.to_lowercase()) else {
        return false;
    };
    if subcommand.starts_with('-') {
        return false;
    }
    let rest = &args[index + 1..];
    if !has_safe_git_arguments(&subcommand, rest) {
        return false;
    }
    match subcommand.as_str() {
        "status" | "log" | "diff" | "show" | "ls-files" => true,
        "branch" => is_safe_git_branch(rest),
        "remote" => is_safe_git_remote(rest),
        "grep" => !rest.iter().any(|argument| {
            long_option_prefix(argument, "--textconv", "--textc")
                || long_option_prefix(argument, "--open-files-in-pager", "--op")
                || long_option_prefix(argument, "--ext-grep", "--ext")
        }),
        _ => false,
    }
}

fn normalize(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    match (path.starts_with('/'), parts.is_empty()) {
        (true, _) => format!("/{}", parts.join("/")),
        (false, true) => ".".to_string(),
        (false, false) => parts.join("/"),
    }
}

fn is_working_directory(directory: &str, cwd: Option<&Path>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    if directory.split(['/', '\\']).any(|part| part == "..") {
        return false;
    }
    let normal = normalize(directory);
    normal == "." || (directory.starts_with('/') && normal == normalize(&cwd.to_string_lossy()))
}

fn has_safe_git_arguments(subcommand: &str, args: &[&str]) -> bool {
    !args.iter().any(|argument| {
        *argument == "--help"
            || is_option(argument, "--show-signature")
            || argument.contains("%G")
            || is_option(argument, "--output")
            || is_option(argument, "--ext-diff")
            || is_option(argument, "--textconv")
            || *argument == "--paginate"
            || is_option(argument, "--open-files-in-pager")
            || (subcommand == "grep" && argument.starts_with("-O"))
    })
}

/// Git accepts any unambiguous prefix of a long option: `--del` is `--delete`.
fn long_option_prefix(argument: &str, option: &str, shortest: &str) -> bool {
    let name = argument.split('=').next().unwrap_or_default();
    name.len() >= shortest.len() && option.starts_with(name)
}

fn is_safe_git_branch(args: &[&str]) -> bool {
    if args.iter().any(|argument| !argument.starts_with('-')) {
        return false;
    }
    !args.iter().any(|argument| {
        let short = argument[1..].split('-').next().unwrap_or_default();
        short.contains(['d', 'D', 'm', 'M', 'c', 'C', 'u'])
            || long_option_prefix(argument, "--delete", "--del")
            || long_option_prefix(argument, "--move", "--mov")
            || long_option_prefix(argument, "--copy", "--cop")
            || long_option_prefix(argument, "--edit-description", "--e")
            || long_option_prefix(argument, "--unset-upstream", "--u")
            || long_option_prefix(argument, "--set-upstream-to", "--set-u")
            || long_option_prefix(argument, "--create-reflog", "--creat")
    })
}

fn is_safe_git_remote(args: &[&str]) -> bool {
    let Some(action_index) = args.iter().position(|argument| !argument.starts_with('-')) else {
        return true;
    };
    match args[action_index] {
        "get-url" => true,
        "show" => {
            let show_args = &args[action_index + 1..];
            if show_args.contains(&"--") {
                return false;
            }
            let remotes = show_args.iter().filter(|argument| !argument.starts_with('-')).count();
            remotes == 0 || (remotes == 1 && show_args.contains(&"-n"))
        }
        _ => false,
    }
}

fn is_safe_gh_command(args: &[&str]) -> bool {
    let (Some(group), Some(action)) = (args.first(), args.get(1)) else {
        return false;
    };
    if group.starts_with('-') || action.starts_with('-') {
        return false;
    }
    let path = format!("{} {}", group.to_lowercase(), action.to_lowercase());
    if !GH_READ_PATHS.contains(&path.as_str()) {
        return false;
    }
    let rest = &args[2..];
    let unsafe_argument = |argument: &str| {
        argument.starts_with("-w")
            || is_option(argument, "--web")
            || is_option(argument, "--browser")
            || argument == "--paginate"
            || is_option(argument, "--pager")
            || is_option(argument, "--output")
    };
    !rest.iter().any(|argument| unsafe_argument(argument)) && has_gh_json_output(rest)
}

/// A read must return JSON: without `--json`, `gh` may open a pager or a browser.
fn has_gh_json_output(args: &[&str]) -> bool {
    let mut has_json = false;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index];
        if argument == "--json" {
            match args.get(index + 1) {
                Some(value) if !value.starts_with('-') => {
                    has_json = true;
                    index += 1;
                }
                _ => return false,
            }
        } else if let Some(value) = argument.strip_prefix("--json=") {
            if value.is_empty() {
                return false;
            }
            has_json = true;
        }
        index += 1;
    }
    has_json
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safe(command: &str) -> bool {
        is_safe_command(command, Some(Path::new("/project")))
    }

    #[test]
    fn ordinary_read_only_commands_are_allowed() {
        for command in [
            "ls -la",
            "cat src/index.ts | head -20",
            "rg 'fn main' src/",
            // Case-insensitive `-i` is a read flag everywhere except sed.
            "grep -n -i \"motion\\|animat\\|reduced\" docs/design.md",
            "rg -i pattern src/",
            "git grep -i pattern",
            "git status",
            "git log --oneline -5",
            "git diff HEAD~1",
            "git --no-pager -C . log -3",
            "git -C /project status",
            "npm test",
            "npm run typecheck",
            "pnpm run lint",
            "cargo test",
            "cargo check --workspace",
            "tsc --noEmit",
            "sed -n 10,20p src/main.rs",
            "echo hello && wc -l README.md",
            "hostname",
        ] {
            assert!(safe(command), "{command}");
        }
    }

    #[test]
    fn anything_that_writes_escapes_the_parser_or_mutates_is_blocked() {
        for command in [
            "rm -rf build",
            "ls > listing.txt",
            "cat a && touch b",
            "git commit -m 'x'",
            "git push",
            "git branch -D old",
            "git branch new-branch",
            "git -C ../other status",
            "git remote add origin x",
            "npm install",
            "npm audit fix",
            "pnpm add lodash",
            "find . -exec rm {} ;",
            "sed -i 's/a/b/' file",
            "sed --in-place 's/a/b/' file",
            "sed -n 's/a/b/p' file",
            "FOO=bar echo hi",
            "ls $(whoami)",
            "ls `pwd`",
            "ls; sleep 1 &",
            "sudo ls",
            "cargo build",
            "cargo run",
            "hostname newname",
            "sort -o out.txt in.txt",
            "",
            "ls\nrm x",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn the_offending_segment_is_named() {
        assert_eq!(blocked_segment("ls && rm -rf x", None).as_deref(), Some("rm -rf x"));
        assert_eq!(blocked_segment("cat a | grep b", None), None);
        assert_eq!(blocked_segment("   ", None).as_deref(), Some("(empty command)"));
    }

    #[test]
    fn named_test_scripts_and_local_test_runners_are_allowed() {
        for command in [
            "pnpm test:web",
            "pnpm test:worker",
            "pnpm test:rust",
            "pnpm run test:unit -- --runInBand",
            "pnpm --dir worker test",
            "pnpm -C 'test fixture' run test:unit",
            "pnpm --dir=worker test:integration",
            "pnpm --filter '@app/core' test",
            "pnpm -F '@app/core' run test:unit",
            "pnpm -r test",
            "pnpm -w check",
            "pnpm exec vitest run src/example.test.ts",
            "pnpm --dir worker exec jest --runInBand",
            "pnpm exec tsc --noEmit",
            "npm run test:unit -- --runInBand",
            "npm --prefix worker test",
            "npm --workspace '@app/core' run test:unit",
            "python3 -m pytest tests/",
            "cargo test --manifest-path src-tauri/Cargo.toml",
        ] {
            assert!(safe(command), "{command}");
        }
    }

    #[test]
    fn test_command_support_stops_at_checks() {
        for command in [
            "pnpm run build",
            "pnpm run testevil",
            "pnpm run test:",
            "pnpm --dir worker add lodash",
            "pnpm --dir",
            "pnpm --dir= test",
            "pnpm --dir --filter test",
            "pnpm --unknown test",
            "pnpm exec rm keep.txt",
            "pnpm exec node script.js",
            "pnpm exec cargo build",
            "pnpm exec tsc --noEmit --incremental",
            "pnpm dlx vitest run",
            "npm exec vitest run",
            "npm --prefix worker audit fix",
            "python3 -m other_module",
            "pnpm test:web --fix",
            "pnpm test:web && rm keep.txt",
            "pnpm test:web > result.txt",
            "pnpm test:web $(touch keep.txt)",
        ] {
            assert!(!safe(command), "{command}");
        }
    }

    #[test]
    fn gh_reads_are_allowed_only_when_they_return_json() {
        assert!(safe("gh pr view 12 --json title,body"));
        assert!(safe("gh issue list --json=number,title"));
        assert!(!safe("gh pr view 12"));
        assert!(!safe("gh pr view 12 --web --json title"));
        assert!(!safe("gh pr merge 12"));
        assert!(!safe("gh pr view 12 --json"));
    }
}
