use std::path::PathBuf;
use std::time::Instant;

use codemap::app::{Options, View};
use codemap::gh::Gh;
use codemap::herdr::{self, Client, PaneOpen, PluginContext};
use codemap::run::{self, Trace};
use codemap::store::Store;
use codemap::theme::Theme;

const HELP: &str = "codeMap: your agents' files, what they changed, and the history around it.

usage:
  codemap [PATH] [--view files|customisations|status|changes|history|branches]
          [--trace-startup]
  codemap open [--view VIEW] [--placement popup|split|tab]   (herdr action)
  codemap ui [--view VIEW]                                   (herdr pane)
  codemap edit                                               (herdr pane: the editor)
  codemap hook                                               (herdr event hook)
  codemap doctor

keys: ? all keys · q or esc closes
";

struct Args {
    cmd: Option<String>,
    path: Option<PathBuf>,
    view: Option<View>,
    placement: String,
    trace: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        cmd: None,
        path: None,
        view: None,
        placement: "popup".into(),
        trace: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "-V" | "--version" => {
                println!("codemap {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--view" | "-v" => {
                let v = it.next().ok_or("--view needs a value")?;
                args.view = Some(View::parse(&v).ok_or(format!("unknown view {v}"))?);
            }
            "--placement" => args.placement = it.next().ok_or("--placement needs a value")?,
            "--trace-startup" => args.trace = true,
            "open" | "ui" | "edit" | "hook" | "doctor"
                if args.cmd.is_none() && args.path.is_none() =>
            {
                args.cmd = Some(a)
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => {
                if args.path.is_some() {
                    return Err(format!("unexpected argument {s}"));
                }
                args.path = Some(PathBuf::from(s));
            }
        }
    }
    Ok(args)
}

fn main() {
    let t0 = Instant::now();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("codemap: {e}\n");
            }
            eprint!("{HELP}");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    let code = match args.cmd.as_deref() {
        Some("hook") => codemap::hook::run(),
        Some("open") => open(&args),
        Some("edit") => edit(),
        Some("doctor") => doctor(),
        _ => ui(args, t0),
    };
    std::process::exit(code);
}

fn view_from(args: &Args) -> View {
    args.view
        .or_else(|| {
            std::env::var("CODEMAP_VIEW")
                .ok()
                .and_then(|v| View::parse(&v))
        })
        .unwrap_or(View::Changes)
}

fn ui(args: Args, t0: Instant) -> i32 {
    let view = view_from(&args);
    let context = if args.path.is_some() {
        None
    } else {
        PluginContext::from_env()
    };
    let client = if herdr::inside_herdr() || std::env::var_os("CODEMAP_CONTEXT_JSON").is_some() {
        Client::from_env()
    } else {
        None
    };
    let opts = Options {
        path: args.path,
        view,
        context,
        client,
        store: Store::from_env(),
        theme: Theme::load(),
        gh: Gh::from_env(),
    };
    let trace = Trace {
        enabled: args.trace || std::env::var_os("CODEMAP_TRACE_FILE").is_some(),
        t0,
    };
    match run::run(opts, trace) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("codemap: {e}");
            1
        }
    }
}

fn open(args: &Args) -> i32 {
    let Some(client) = Client::from_env() else {
        eprintln!("codemap open: HERDR_SOCKET_PATH is not set (run it from herdr)");
        return 1;
    };
    let view = view_from(args);
    let mut env = vec![("CODEMAP_VIEW".to_string(), view.name().to_string())];
    let ctx_json = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok();
    if let Some(json) = &ctx_json {
        env.push(("CODEMAP_CONTEXT_JSON".to_string(), json.clone()));
    }
    let ctx = ctx_json.as_deref().and_then(PluginContext::parse);
    let placement = args.placement.clone();
    let mut req = PaneOpen {
        plugin_id: herdr::PLUGIN_ID.to_string(),
        entrypoint: "ui".to_string(),
        placement: placement.clone(),
        width: Some("94%".into()),
        height: Some("92%".into()),
        target_pane_id: ctx.as_ref().and_then(|c| c.focused_pane_id.clone()),
        cwd: None,
        env,
        focus: true,
    };
    match client.plugin_pane_open(&req) {
        Ok(_) => 0,
        Err(e) if e.code == "ui_busy" && placement == "popup" => {
            req.placement = "overlay".into();
            match client.plugin_pane_open(&req) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("codemap open: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("codemap open: {e}");
            1
        }
    }
}

fn edit() -> i32 {
    use std::os::unix::process::CommandExt;

    let Ok(path) = std::env::var(codemap::open::EDIT_PATH_ENV) else {
        eprintln!(
            "codemap edit: {} is not set (herdr runs this from the popup)",
            codemap::open::EDIT_PATH_ENV
        );
        return 2;
    };
    let line = std::env::var(codemap::open::EDIT_LINE_ENV)
        .ok()
        .and_then(|l| l.parse().ok());
    let Some(mut cmd) = codemap::open::editor_command(std::path::Path::new(&path), line) else {
        eprintln!("codemap edit: no editor to run");
        return 1;
    };
    let failure = cmd.exec();
    eprintln!("codemap edit: {failure}");
    1
}

fn doctor() -> i32 {
    let store = Store::from_env();
    println!("codemap {}", env!("CARGO_PKG_VERSION"));
    println!("inside herdr:   {}", herdr::inside_herdr());
    println!(
        "herdr socket:   {}",
        std::env::var("HERDR_SOCKET_PATH").unwrap_or_else(|_| "(not set)".into())
    );
    if let Some(client) = Client::from_env() {
        match client.ping() {
            Ok(v) => println!(
                "herdr server:   {} (protocol {})",
                v.get("version").and_then(|x| x.as_str()).unwrap_or("?"),
                v.get("protocol").and_then(|x| x.as_u64()).unwrap_or(0)
            ),
            Err(e) => println!("herdr server:   unreachable ({e})"),
        }
    }
    println!(
        "herdr config:   {}",
        codemap::theme::herdr_config_path().display()
    );
    let t = Theme::load();
    println!("theme:          {}", t.name);
    println!("editor:         {}", codemap::open::editor());
    println!("state dir:      {}", store.root.display());
    let cwd = std::env::current_dir().unwrap_or_default();
    match codemap::git::discover(&cwd) {
        Ok(r) => {
            println!("repository:     {}", r.root.display());
            let panes = store.panes_for_repo(&r.root);
            println!("checkpoints:    {} agent(s) with turns here", panes.len());
        }
        Err(_) => println!("repository:     (none at {})", cwd.display()),
    }
    0
}
