//! `wack`: the agent without the app. It shares the app's data folder, so a connection added
//! here is the one the app uses, and a debug build uses the dev app's folder.
//!
//! API keys are typed at a hidden prompt (or piped in) and go straight to the secret store. No
//! command prints one, and none takes one as an argument, where it would land in shell history.

mod args;
mod print;
mod terminal;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use args::Args;
use wack_agent::{Agent, Control, Outcome, Profile, Registry, Session, Settings};
use wack_core::{Model, NewConnection, Store};
use wack_llm::{Effort, WireFormat};

const HELP: &str = "wack: the WackCode agent without the app

  wack connect                 Add a provider connection. Asks for what it needs; the key is typed hidden.
      --name <name>  --format anthropic|openai-responses|openai-chat  --base-url <url>
      --model <id> (repeatable)  --context-window <tokens>
  wack connections             List connections.
  wack disconnect <name>       Remove a connection and its key.
  wack run [options] <prompt>  Run the agent in the current folder.
      --connection <name>      Which connection to use (needed when there are several).
      --model <id>             Defaults to the connection's first model.
      --effort low|medium|high|xhigh|max
      --cwd <folder>           Work in another folder.
      --resume <session file>  Continue an earlier run.

  --data-dir <folder>          Use another data folder (for tests).

While a run is going, type a line and press Return to send the agent a message. Ctrl-C stops the run.";

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
    let models = models.into_iter().map(|id| Model { id, context_window }).collect();
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
        println!("{}\n  {}  {}\n  models: {}", connection.name, format_name(connection.format), connection.base_url, models.join(", "));
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

async fn agent(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    let wanted = args.value("--connection")?;
    let model = args.value("--model")?;
    let effort = match args.value("--effort")? {
        Some(effort) => Some(Effort::parse(&effort).ok_or("The effort must be low, medium, high, xhigh or max.")?),
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
    let connection = match &wanted {
        Some(name) => store.connection(name).ok_or_else(|| format!("There is no connection named {name}."))?,
        None => match store.connections() {
            [] => return Err("No connections yet. Add one with: wack connect".to_string()),
            [only] => only,
            several => {
                let names: Vec<&str> = several.iter().map(|connection| connection.name.as_str()).collect();
                return Err(format!("Choose a connection with --connection: {}", names.join(", ")));
            }
        },
    };
    let model = match model {
        Some(id) => connection.models.iter().find(|model| model.id == id).cloned().unwrap_or(Model { id, context_window: None }),
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
    let registry = Arc::new(Registry::new(wack_tools::core_tools())?);
    let settings = Settings { model: model.id.clone(), effort, max_output_tokens: None, context_window: model.context_window };
    let mut agent = Agent::new(Arc::new(store.provider(connection)?), registry, session, &Profile::code(), settings);
    eprintln!("{}", print::dim(&format!("{} · {} · {}", connection.name, model.id, agent.session().cwd().display())));

    let control = Control::new();
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
    if !piped && terminal::interactive() {
        let steering = control.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    steering.steer(line);
                }
            }
        });
    }

    let printer = print::Printer::new();
    let outcome = agent.run(&prompt, &control, &|event| printer.event(event)).await;
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
