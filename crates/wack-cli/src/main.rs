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
use wack_agent::{Agent, AgentMode, Agents, Control, Kit, Outcome, Profile, Registry, Session, Settings, Tasks};
use wack_core::{
    Connection, Model, NewConnection, Reasoning, Resolved, Store,
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
      --chat                   Run the Chat area's assistant instead: no project, a fresh
                               scratchpad of its own, and the Chat area's tools and memory.
      --resume <session file>  Continue an earlier run.
                               A prompt starting with /name runs that skill or command, as in the app.
  wack skills                  List the skills a run loads, and any that could not load.
  wack commands                List the slash commands a run understands.
  wack agents                  List the sub-agents a run can start (Settings › Sub-agents).
  wack mcp list                List the MCP servers (Settings › MCP servers) and the projects they reach.
  wack mcp test <name>         Connect to one, list its tools and close. Starts in --cwd, or here.

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
        Some("agents") => list_agents(args, &folder),
        Some("mcp") => mcp(args, &folder).await,
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

fn list_agents(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    args.finish()?;
    let store = Store::open(folder)?;
    let config = store.agents_config();
    let catalog = store.agent_catalog();
    if config.delegation == wack_agent::Delegation::Off {
        println!("Sub-agents are off (Settings › Sub-agents).");
    }
    for agent in &catalog.agents {
        let access = if agent.edits() { "edits files" } else { "no edits" };
        let file = agent.file.as_ref().map(|file| format!("\n  {}", file.display())).unwrap_or_default();
        println!("{}  ·  {}  ·  {access}  ·  {} model\n  {}{file}", agent.name, agent.source, agent.model.as_str(), agent.description.lines().next().unwrap_or(""));
    }
    for name in &config.off {
        println!("{name}  (switched off)");
    }
    for lost in &catalog.shadowed {
        println!("{}  ·  {}  (not loaded: {} has an agent with this name)", lost.agent.name, lost.agent.source, lost.by);
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

async fn mcp(args: &mut Args, folder: &Path) -> Result<ExitCode, String> {
    let action = args.command();
    let cwd = args.value("--cwd")?;
    let store = Store::open(folder)?;
    match action.as_deref() {
        Some("list") => {
            args.finish()?;
            if store.mcp_servers().is_empty() {
                println!("No MCP servers yet. Add one in Settings › MCP servers.");
            }
            for server in store.mcp_servers() {
                let reach = match &server.projects {
                    wack_core::mcp::Projects::All => "all projects".to_string(),
                    wack_core::mcp::Projects::Only(ids) => {
                        let names: Vec<&str> = ids.iter().filter_map(|id| store.project(id)).map(|project| project.name.as_str()).collect();
                        if names.is_empty() { "no projects".to_string() } else { names.join(", ") }
                    }
                };
                let what = if server.transport == wack_mcp::Transport::Stdio { std::iter::once(server.command.as_str()).chain(server.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ") } else { server.url.clone() };
                println!(
                    "{}  ·  {}{}  ·  {}\n  {what}\n  {} tools known, {} switched off",
                    server.name,
                    server.transport.label(),
                    if server.enabled { "" } else { " · off" },
                    reach,
                    server.tools.len(),
                    server.disabled_tools.len()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Some("test") => {
            let name = args.rest().join(" ");
            if name.trim().is_empty() {
                return Err("Name the server to test: wack mcp test <name>".into());
            }
            let id = store.mcp_servers().iter().find(|server| server.name.eq_ignore_ascii_case(name.trim())).map(|server| server.id.clone()).ok_or_else(|| format!("There is no MCP server named {}.", name.trim()))?;
            let spec = store.mcp_spec(&id).ok_or("That MCP server no longer exists.")?;
            let cwd = match cwd {
                Some(cwd) => PathBuf::from(cwd),
                None => std::env::current_dir().map_err(|error| format!("Could not find the current folder: {error}"))?,
            };
            let hub = Servers(wack_mcp::Hub::new(Some(store.mcp_cache_folder())));
            eprintln!("{}", print::dim(&format!("Connecting to {}…", spec.name)));
            let tools = hub.0.probe(&spec, &cwd).await?;
            let mut store = store;
            store.set_mcp_tools(&id, &spec.fingerprint(), &tools)?;
            println!("{} tool{}:", tools.len(), if tools.len() == 1 { "" } else { "s" });
            for tool in &tools {
                let about = tool.description.as_deref().unwrap_or_default().lines().next().unwrap_or_default();
                println!("  {}{}  {about}", wack_mcp::tool_name(&spec.slug, &tool.name), if tool.read_only() { " (read-only)" } else { "" });
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Err("Use wack mcp list, or wack mcp test <name>.".into()),
    }
}

/// `wack run`'s MCP servers, ended however the run ends: `process::exit` at the end calls
/// `terminate_all` itself, and an early error drops this.
struct Servers(wack_mcp::Hub);

impl Drop for Servers {
    fn drop(&mut self) {
        self.0.terminate_all();
    }
}

/// `wack run --chat`: the Chat area's agent, built exactly as the app builds it
/// (`wack_core::chat_agent`), in a scratchpad of its own beside the data folder's others.
async fn chat_run(store: &Store, connection: Connection, model: Model, effort: Option<Reasoning>, resume: Option<String>, prompt: String, piped: bool) -> Result<ExitCode, String> {
    if let Some(wanted) = effort
        && !model.choices().contains(&wanted)
    {
        let offered: Vec<&str> = model.choices().iter().map(|level| level.label()).collect();
        return Err(format!("{} does not list {} reasoning. It offers: {}.", model.id, wanted.label(), offered.join(", ")));
    }
    let session = match resume {
        Some(path) => Session::open(Path::new(&path))?,
        None => {
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis()).unwrap_or(0);
            let scratch = store.sessions_folder().parent().map(|data| data.join("scratch").join(format!("cli-{stamp:x}"))).ok_or("The data folder has no parent.")?;
            std::fs::create_dir_all(&scratch).map_err(|error| format!("Could not make the scratchpad: {error}"))?;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o700));
            Session::create(&store.sessions_folder().join(format!("{stamp:x}.jsonl")), &scratch)?
        }
    };
    let stem = session.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let usage: Arc<dyn wack_agent::Recorder> =
        Arc::new(wack_core::RunRecorder::new(store.usage(), &connection, &stem, None, session.cwd(), wack_core::ChatKind::Chat));
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
    let hub = Arc::new(Servers(wack_mcp::Hub::new(Some(store.mcp_cache_folder()))));
    let starting: wack_mcp::Starting = Arc::new(|servers: Vec<String>| {
        if !servers.is_empty() {
            eprintln!("{}", print::dim(&format!("Starting MCP servers: {}…", servers.join(", "))));
        }
    });
    let mcp_folder = store.chat_area_folder()?;
    let mcp = wack_mcp::assemble(&hub.0, &store.mcp_specs_for(&mcp_folder), &mcp_folder, &control.stop_token(), starting).await;
    let effort = model.pick(effort);
    let settings = Settings {
        model: model.id.clone(),
        effort: model.effort_for(effort),
        max_output_tokens: None,
        context_window: model.context_window,
        accepts_images: model.accepts_images,
        ultra: effort.is_ultra(),
        commit_attribution: false,
    };
    let parts = wack_core::ChatParts {
        provider: Arc::new(store.provider(&connection)?),
        session,
        settings,
        memory: store.chat_area_memory(),
        web: store.web_setup(),
        browser: None,
        canvases: None,
        mcp,
        usage: store.usage_recording().then_some(usage),
        readable: Vec::new(),
    };
    let mut agent = wack_core::chat_agent(parts)?;
    let mut header = format!("{} · {} · Chat · {}", connection.name, model.id, agent.session().cwd().display());
    if effort.is_ultra() {
        header.push_str(" · Ultra");
    }
    eprintln!("{}", print::dim(&header));
    if listening {
        let (steering, asking) = (control.clone(), asking.clone());
        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                if !asking.answer(&line, &steering) && !line.trim().is_empty() {
                    steering.steer(line);
                }
            }
        });
    }
    let printer = print::Printer::new(asking);
    let outcome = agent.run(&prompt, &control, &|event| printer.event(event)).await;
    store.usage().flush();
    hub.0.terminate_all();
    printer.finish();
    eprintln!("{}", print::dim(&format!("{}\nsession: {}", printer.usage_line(), agent.session().path().display())));
    std::process::exit(match outcome {
        Ok(Outcome::Done) => 0,
        Ok(Outcome::Stopped) => {
            eprintln!("Stopped.");
            130
        }
        Ok(Outcome::CutOff | Outcome::Refused) => 1,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    })
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
    let chat = args.flag("--chat");
    if chat && (mode.is_some() || cwd.is_some()) {
        return Err("--chat has no project, so it takes neither --mode nor --cwd.".into());
    }
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

    if chat {
        return chat_run(&store, connection.clone(), model, effort, resume, prompt, piped).await;
    }
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
    let mut kit = Kit::new(&session, mode, store.planning_unrestricted(), store.memory_for(session.cwd()))
        .with_skills(&store.skill_catalog())
        .with_tasks(&session, tasks.clone());
    // Sub-agents run on this run's model unless their role asks for the helper.
    let agents = Agents::new();
    let stem = session.path().file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let resolved = Resolved { connection: connection.clone(), effort: model.pick(effort), model: model.clone() };
    if let Some(setup) = store.agents_setup(store.child_model(&resolved, &stem, session.cwd())?, &stem, session.cwd(), agents.clone()) {
        kit = kit.with_agents(&session, setup);
        print::watch_agents(&agents);
    }
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
    // Only a person at the keyboard can answer a question.
    let listening = !piped && terminal::interactive();
    let control = if listening { Control::with_user() } else { Control::new() };
    let asking = Arc::new(asking::Asking::default());
    let stopper = control.clone();
    let children = agents.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nStopping. Press Ctrl-C again to quit at once.");
            stopper.stop();
            children.stop_all();
        }
        if tokio::signal::ctrl_c().await.is_ok() {
            std::process::exit(130);
        }
    });

    // MCP servers get a hub of their own, ended with the run. Only a server with no tool list
    // yet is waited for, and Ctrl-C stops the wait.
    let hub = Arc::new(Servers(wack_mcp::Hub::new(Some(store.mcp_cache_folder()))));
    let starting: wack_mcp::Starting = Arc::new(|servers: Vec<String>| {
        if !servers.is_empty() {
            eprintln!("{}", print::dim(&format!("Starting MCP servers: {}…", servers.join(", "))));
        }
    });
    let mcp = wack_mcp::assemble(&hub.0, &store.mcp_specs_for(session.cwd()), session.cwd(), &control.stop_token(), starting).await;
    let mut tools = wack_tools::core_tools();
    tools.extend(kit.tools());
    tools.extend(wack_web::tools(&store.web_setup())?);
    tools.extend(mcp.tools);
    let agent_tools = kit.agent_tools(&tools);
    tools.extend(agent_tools);
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
        accepts_images: model.accepts_images,
        ultra: effort.is_ultra(),
        commit_attribution: store.git_config().commit_attribution,
    };
    let agent = Agent::new(Arc::new(store.provider(connection)?), registry, session, &Profile::code(), settings);
    let agent = match usage {
        Some(usage) => agent.with_recorder(usage),
        None => agent,
    };
    let agent = match mcp.feature {
        Some(feature) => agent.with_feature(Box::new(feature)),
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
    // Sub-agents in the background wake it the same way; their reports don't count against the
    // cap, which their own start budget bounds.
    let news = || tasks.has_news() || agents.has_news();
    let working = || tasks.running().len() + agents.active().len();
    while matches!(outcome, Ok(Outcome::Done)) && !control.stopped() {
        if !news() {
            let running = working();
            if running == 0 {
                break;
            }
            printer.finish();
            eprintln!("{}", print::dim(&format!("Waiting on {running} in the background — Ctrl-C stops {}.", if running == 1 { "it" } else { "them" })));
            while !news() && !control.stopped() && working() > 0 {
                tokio::select! {
                    () = tasks.news() => {}
                    () = agents.news() => {}
                    () = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
                }
            }
            if control.stopped() {
                agents.stop_all();
            }
            if !news() {
                break;
            }
        }
        if !agents.has_news() {
            if wakes == MAX_WAKES {
                eprintln!("{}", print::dim(&format!("Background commands reported {MAX_WAKES} times in a row; stopping here.")));
                break;
            }
            wakes += 1;
        }
        outcome = agent.wake(&control, &|event| printer.event(event)).await;
    }
    agents.stop_all();
    tasks.terminate_all();
    hub.0.terminate_all();
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
