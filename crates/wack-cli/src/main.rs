//! `wack`: the agent without the app. It shares the app's data folder, so a connection added
//! here is the one the app uses, and a debug build uses the dev app's folder.
//!
//! API keys are typed at a hidden prompt (or piped in) and go straight to the secret store. No
//! command prints one, and none takes one as an argument, where it would land in shell history.

mod args;
mod asking;
mod print;
mod terminal;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use args::Args;
use wack_agent::{Agent, AgentMode, Control, Kit, Outcome, Profile, Registry, Session, Settings, Tasks};
use wack_core::{
    Connection, Model, NewConnection, Reasoning, Store,
    skills::{self, Outgoing},
};
use wack_llm::WireFormat;

const HELP: &str = "wack: the WackCode agent without the app

  wack connect                 Add a provider connection. Asks for what it needs; the key is typed hidden.
      --name <name>  --format anthropic|openai-responses|openai-chat  --base-url <url>
      --model <id> (repeatable)  --context-window <tokens>
  wack connections             List connections.
  wack disconnect <name>       Remove a connection and its key.
  wack run [options] <prompt>  Run the agent in the current folder.
      --connection <name>      Which connection to use (needed when there are several).
      --model <id>             Defaults to the connection's first model.
      --effort off|minimal|low|medium|high|xhigh|max|ultra
      --mode build|plan|ultraplan   Plan and Ultra Plan only read, ask, and end with a plan.
                               A resumed run keeps its mode unless you give one.
      --cwd <folder>           Work in another folder.
      --resume <session file>  Continue an earlier run.
                               A prompt starting with /name runs that skill or command, as in the app.
  wack skills                  List the skills a run loads, and any that could not load.
  wack commands                List the slash commands a run understands.

  --data-dir <folder>          Use another data folder (for tests).

While a run is going, type a line and press Return to send the agent a message, or to answer a
question it asked. Ctrl-C stops the run.";

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = Args::new(std::env::args().skip(1));
    let result = match run(&mut args).await {
        Ok(code) => return code,
        Err(message) => message,
    };
    eprintln!("{result}");
    ExitCode::FAILURE
}

async fn run(args: &mut Args) -> Result<ExitCode, String> {
    let folder = match args.value("--data-dir")? {
        Some(folder) => PathBuf::from(folder),
        None => Store::default_folder()?,
    };
    match args.command().as_deref() {
        Some("connect") => connect(args, &folder),
        Some("connections") => connections(&folder),
        Some("disconnect") => disconnect(args, &folder),
        Some("run") => agent(args, &folder).await,
        Some("skills") => list_skills(args, &folder),
        Some("commands") => list_commands(args, &folder),
        None | Some("help" | "--help" | "-h") => {
            println!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(format!("There is no command named {other}. Run wack help.")),
    }
}

fn parse_format(text: &str) -> Option<WireFormat> {
    Some(match text.trim().to_ascii_lowercase().as_str() {
        "1" | "anthropic" | "anthropic-messages" => WireFormat::AnthropicMessages,
        "2" | "openai-responses" | "responses" => WireFormat::OpenaiResponses,
        "3" | "openai-chat" | "chat" | "compatible" | "openai-compatible" => WireFormat::OpenaiChat,
        _ => return None,
    })
}

fn format_name(format: WireFormat) -> &'static str {
    match format {
        WireFormat::AnthropicMessages => "anthropic",
        WireFormat::OpenaiResponses => "openai-responses",
        WireFormat::OpenaiChat => "openai-chat",
    }
}

fn connect(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    let name = match args.value("--name")? {
        Some(name) => name,
        None => terminal::ask("Name")?,
    };
    let format = match args.value("--format")? {
        Some(format) => format,
        None => terminal::ask("Type: 1 Anthropic, 2 OpenAI Responses, 3 OpenAI-compatible chat completions")?,
    };
    let format = parse_format(&format).ok_or("The type must be anthropic, openai-responses or openai-chat.")?;
    let base_url = match args.value("--base-url")? {
        Some(url) => url,
        None => {
            let usual = match format {
                WireFormat::AnthropicMessages => "https://api.anthropic.com",
                _ => "https://api.openai.com/v1",
            };
            let typed = terminal::ask(&format!("Address (Return for {usual})"))?;
            if typed.is_empty() { usual.to_string() } else { typed }
        }
    };
    let mut models = args.values("--model")?;
    if models.is_empty() {
        models = terminal::ask("Model ids, separated by commas")?.split(',').map(str::to_string).collect();
    }
    let context_window = match args.value("--context-window")? {
        Some(window) => Some(window),
        None if terminal::interactive() => Some(terminal::ask("Context window in tokens (Return to skip)")?),
        None => None,
    };
    let context_window = match context_window.as_deref().map(str::trim).filter(|window| !window.is_empty()) {
        Some(window) => Some(window.replace(['_', ','], "").parse::<u64>().map_err(|_| "The context window must be a number of tokens.")?),
        None => None,
    };
    args.finish()?;

    let key = terminal::ask_hidden("API key (hidden)")?;
    let mut store = Store::open(folder)?;
    // Reasoning efforts are picked in Settings › Providers; a model added here does not reason.
    let models = models.into_iter().map(|id| Model::new(id, context_window)).collect();
    let added = store.add_connection(NewConnection { name, format, base_url, models }, &key)?;
    println!("Added {} ({}, {}).", added.name, format_name(added.format), added.base_url);
    Ok(ExitCode::SUCCESS)
}

fn connections(folder: &Path) -> Result<ExitCode, String> {
    let store = Store::open(folder)?;
    if store.connections().is_empty() {
        println!("No connections yet. Add one with: wack connect");
    }
    for connection in store.connections() {
        let models: Vec<&str> = connection.models.iter().map(|model| model.id.as_str()).collect();
        let off = if connection.enabled { "" } else { "  (switched off)" };
        println!("{}{off}\n  {}  {}\n  models: {}", connection.name, format_name(connection.format), connection.base_url, models.join(", "));
    }
    Ok(ExitCode::SUCCESS)
}

fn disconnect(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    let name = args.rest().join(" ");
    let mut store = Store::open(folder)?;
    let connection = store.connection(&name).ok_or_else(|| format!("There is no connection named {name}."))?.clone();
    store.remove_connection(&connection.id)?;
    println!("Removed {}.", connection.name);
    Ok(ExitCode::SUCCESS)
}

fn list_skills(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    args.finish()?;
    let store = Store::open(folder)?;
    let catalog = store.skill_catalog();
    if catalog.skills.is_empty() {
        println!("No skills yet. Put one in {} (a folder with a SKILL.md).", store.skills_library().display());
    }
    for skill in &catalog.skills {
        let manual = if skill.manual { "  (only as /name)" } else { "" };
        println!("{}{manual}  ·  {}\n  {}\n  {}", skill.name, skill.source, skill.description.lines().next().unwrap_or(""), skill.file.display());
    }
    for lost in &catalog.shadowed {
        println!("{}  ·  {}  (not loaded: {} has a skill with this name)\n  {}", lost.skill.name, lost.skill.source, lost.by, lost.skill.file.display());
    }
    for problem in &catalog.problems {
        println!("{}\n  not loaded: {}", problem.file.display(), problem.message);
    }
    Ok(ExitCode::SUCCESS)
}

fn list_commands(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    args.finish()?;
    let store = Store::open(folder)?;
    for command in store.command_catalog() {
        let hint = command.hint.as_deref().map(|hint| format!(" {hint}")).unwrap_or_default();
        println!("/{}{hint}  ·  {}\n  {}", command.name, command.source, command.description.lines().next().unwrap_or(""));
    }
    Ok(ExitCode::SUCCESS)
}

/// Times background news may wake the agent before `wack run` gives up.
const MAX_WAKES: u32 = 3;

async fn agent(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    let wanted = args.value("--connection")?;
    let model = args.value("--model")?;
    let effort = match args.value("--effort")? {
        Some(effort) => Some(Reasoning::parse(&effort).ok_or("The effort must be off, minimal, low, medium, high, xhigh, max or ultra.")?),
        None => None,
    };
    let mode = match args.value("--mode")? {
        Some(mode) => Some(AgentMode::parse(&mode).ok_or("The mode must be build, plan or ultraplan.")?),
        None => None,
    };
    let cwd = args.value("--cwd")?;
    let resume = args.value("--resume")?;
    let mut prompt = args.rest().join(" ");
    let piped = prompt.trim().is_empty() && !terminal::interactive();
    if piped {
        prompt = std::io::read_to_string(std::io::stdin()).map_err(|error| format!("Could not read the prompt: {error}"))?;
    }
    if prompt.trim().is_empty() {
        return Err("Give the agent something to do: wack run \"your request\"".to_string());
    }

    let store = Store::open(folder)?;
    let on: Vec<&Connection> = store.enabled_connections().collect();
    let connection = match &wanted {
        Some(name) => match store.connection(name) {
            None => return Err(format!("There is no connection named {name}.")),
            Some(connection) if !connection.enabled => return Err(format!("{} is switched off in Settings › Providers.", connection.name)),
            Some(connection) => connection,
        },
        None => match on.as_slice() {
            [] if store.connections().is_empty() => return Err("No connections yet. Add one with: wack connect".to_string()),
            [] => return Err("Every connection is switched off. Switch one on in Settings › Providers.".to_string()),
            [only] => *only,
            several => {
                let names: Vec<&str> = several.iter().map(|connection| connection.name.as_str()).collect();
                return Err(format!("Choose a connection with --connection: {}", names.join(", ")));
            }
        },
    };
    let model = match model {
        Some(id) => connection.models.iter().find(|model| model.id == id).cloned().unwrap_or(Model::new(id, None)),
        None => connection.models.first().cloned().ok_or("This connection has no models.")?,
    };

    let session = match resume {
        Some(path) => Session::open(Path::new(&path))?,
        None => {
            let cwd = match cwd {
                Some(cwd) => PathBuf::from(cwd),
                None => std::env::current_dir().map_err(|error| format!("Could not find the current folder: {error}"))?,
            };
            let cwd = cwd.canonicalize().map_err(|error| format!("Could not open {}: {error}", cwd.display()))?;
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis()).unwrap_or(0);
            Session::create(&store.sessions_folder().join(format!("{stamp:x}.jsonl")), &cwd)?
        }
    };
    // A leading /name expands as the app's would. The app's own commands have nothing to act on
    // here, except /init, which is a prompt.
    let (prompt, command) = match skills::resolve_message(&store.command_catalog(), prompt.trim(), session.cwd())? {
        Outgoing::Message(_) => (prompt, None),
        Outgoing::Prompt { text, command } => (text, Some(command)),
        Outgoing::Builtin { name, .. } => return Err(format!("/{name} only works in the app.")),
    };
    // Commands the agent runs in the background write their output beside the sessions, in a
    // folder of this session's own.
    let stem = session.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "run".into());
    let tasks_folder = store.sessions_folder().parent().map(|data| data.join("tasks").join(format!("cli-{stem}"))).unwrap_or_else(|| std::env::temp_dir().join(&stem));
    let tasks = Tasks::new(tasks_folder);
    let kit = Kit::new(&session, mode, store.planning_unrestricted(), store.memory_for(session.cwd()))
        .with_skills(&store.skill_catalog())
        .with_tasks(&session, tasks.clone());
    // The usage ledger records this run under the session file's stem, the closest thing to a
    // chat it has, and a write that fails is retried while the run lasts.
    let stem = session.path().file_stem().map(|stem| stem.to_string_lossy().into_owned());
    let usage = stem.map(|stem| store.usage_recorder(connection, &stem, session.cwd()));
    if store.usage_recording() {
        let writer = store.usage().clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                writer.flush();
            }
        });
    }
    let mut tools = wack_tools::core_tools();
    tools.extend(kit.tools());
    let registry = Arc::new(Registry::new(tools)?);
    // The effort must be one the model lists in Settings › Providers; without --effort, the
    // app's default for it.
    if let Some(wanted) = effort
        && !model.choices().contains(&wanted)
    {
        let offered: Vec<&str> = model.choices().iter().map(|level| level.label()).collect();
        return Err(format!("{} does not list {} reasoning. It offers: {}.", model.id, wanted.label(), offered.join(", ")));
    }
    let effort = model.pick(effort);
    let settings = Settings {
        model: model.id.clone(),
        effort: model.effort_for(effort),
        max_output_tokens: None,
        context_window: model.context_window,
        ultra: effort.is_ultra(),
    };
    let agent = Agent::new(Arc::new(store.provider(connection)?), registry, session, &Profile::code(), settings);
    let agent = match usage {
        Some(usage) => agent.with_recorder(usage),
        None => agent,
    };
    let mut agent = kit.attach(agent);
    let mut header = format!("{} · {} · {}", connection.name, model.id, agent.session().cwd().display());
    if kit.plan.mode().planning() {
        header.push_str(&format!(" · {}", kit.plan.mode().label()));
    }
    if effort.is_ultra() {
        header.push_str(" · Ultra");
    }
    eprintln!("{}", print::dim(&header));

    // Only a person at the keyboard can answer a question.
    let listening = !piped && terminal::interactive();
    let control = if listening { Control::with_user() } else { Control::new() };
    let asking = Arc::new(asking::Asking::default());
    let stopper = control.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nStopping. Press Ctrl-C again to quit at once.");
            stopper.stop();
        }
        if tokio::signal::ctrl_c().await.is_ok() {
            std::process::exit(130);
        }
    });
    if listening {
        let (steering, asking) = (control.clone(), asking.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                // A line answers the question on screen, if there is one.
                if !asking.answer(&line, &steering) && !line.trim().is_empty() {
                    steering.steer(line);
                }
            }
        });
    }

    let printer = print::Printer::new(asking);
    let mut outcome = agent.run_as(&prompt, command, &control, &|event| printer.event(event)).await;
    // While commands run in the background the agent is not done: each report wakes it, as the
    // app would, up to MAX_WAKES times.
    let mut wakes = 0;
    while matches!(outcome, Ok(Outcome::Done)) && !control.stopped() {
        if !tasks.has_news() {
            let running = tasks.running().len();
            if running == 0 {
                break;
            }
            printer.finish();
            eprintln!("{}", print::dim(&format!("Waiting on {running} background command{} — Ctrl-C stops {}.", if running == 1 { "" } else { "s" }, if running == 1 { "it" } else { "them" })));
            while !tasks.has_news() && !control.stopped() && !tasks.running().is_empty() {
                tokio::select! {
                    () = tasks.news() => {}
                    () = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                }
            }
            if !tasks.has_news() {
                break;
            }
        }
        if wakes == MAX_WAKES {
            eprintln!("{}", print::dim(&format!("Background commands reported {MAX_WAKES} times in a row; stopping here.")));
            break;
        }
        wakes += 1;
        outcome = agent.wake(&control, &|event| printer.event(event)).await;
    }
    tasks.terminate_all();
    printer.finish();
    eprintln!("{}", print::dim(&format!("{}\nsession: {}", printer.usage_line(), agent.session().path().display())));
    // The steering thread is blocked reading the terminal, so the process is ended directly.
    std::process::exit(match outcome {
        Ok(Outcome::Done) => 0,
        Ok(Outcome::Stopped) => {
            eprintln!("Stopped.");
            130
        }
        Ok(Outcome::CutOff) => {
            eprintln!("The reply reached its length limit before it was finished.");
            1
        }
        Ok(Outcome::Refused) => {
            eprintln!("The model declined to continue.");
            1
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    })
}
