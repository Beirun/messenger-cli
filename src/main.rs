use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use messenger_cli::{
    auth::{self, CookieJar},
    browser,
    client::{Client, SessionState},
    login,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "messenger",
    version,
    about = "Unofficial Messenger client for a personal account"
)]
struct Cli {
    /// Print JSON instead of text
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in through Chrome and save the session cookies
    Login,
    /// Check the saved session against Messenger
    Status,
    /// List recent chats
    Chats {
        /// How many chats to list
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: usize,
    },
    /// Read messages from a chat
    Read {
        /// Chat or thread id
        chat_id: String,
        /// How many messages to return
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
        #[arg(long, value_enum, default_value_t = Backend::Gateway)]
        backend: Backend,
    },
    /// Send a text message to a chat
    Send {
        /// Chat or thread id
        chat_id: String,
        /// Message text, or - to read it from stdin
        text: String,
        #[arg(long, value_enum, default_value_t = Backend::Gateway)]
        backend: Backend,
        /// Send without asking for confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// React to a message in a group chat
    React {
        /// Chat or thread id
        chat_id: String,
        /// Message id, as printed by read
        message_id: String,
        /// One unicode emoji
        emoji: String,
    },
    /// Send a file, image, video or audio to a chat
    Attach {
        /// Chat or thread id
        chat_id: String,
        /// Path to the file
        path: PathBuf,
        /// Caption to send with the file
        #[arg(short = 'm', long)]
        caption: Option<String>,
        /// Send without asking for confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Backend {
    Gateway,
    Browser,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Browser => "browser",
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Login => login::run().await,
        Command::Status => status(cli.json).await,
        Command::Chats { limit } => chats(cli.json, limit).await,
        Command::Read {
            chat_id,
            limit,
            backend,
        } => read(cli.json, chat_id, limit, backend).await,
        Command::Send {
            chat_id,
            text,
            backend,
            yes,
        } => send(cli.json, chat_id, text, backend, yes).await,
        Command::React {
            chat_id,
            message_id,
            emoji,
        } => react(cli.json, chat_id, message_id, emoji).await,
        Command::Attach {
            chat_id,
            path,
            caption,
            yes,
        } => attach(cli.json, chat_id, path, caption, yes).await,
    }
}

async fn status(json: bool) -> Result<()> {
    let path = auth::cookies_path()?;
    if !path.exists() {
        bail!(
            "no cookie file at {}; run `messenger login`",
            path.display()
        );
    }
    let jar = CookieJar::load(&path)?;
    let missing = jar.missing_required();
    let count = jar.len();
    let user = jar.user_id().map(str::to_owned);

    let mut lines = vec![
        format!("cookie file: {}", path.display()),
        format!("cookies loaded: {count}"),
        format!("user id: {}", user.as_deref().unwrap_or("unknown")),
    ];
    let mut session = Value::Null;
    let mut tokens = false;
    if missing.is_empty() {
        lines.push("required cookies: c_user, xs present".to_string());
        let report = Client::new(jar)?
            .check_session()
            .await
            .context("checking the session")?;
        lines.push(format!(
            "session: {} (HTTP {}, {} KB)",
            report.state.label(),
            report.status,
            report.bytes / 1024
        ));
        tokens = report.tokens.is_some();
        if report.state == SessionState::Active {
            lines.push(format!(
                "api tokens: {}",
                if tokens { "found" } else { "missing" }
            ));
        }
        session = json!({
            "state": report.state.label(),
            "http": report.status,
            "bytes": report.bytes,
        });
    } else {
        lines.push(format!("required cookies missing: {}", missing.join(", ")));
        lines.push("session: not checked".to_string());
    }

    if json {
        return print_json(&json!({
            "cookie_file": path,
            "cookies": count,
            "user_id": user,
            "missing": missing,
            "session": session,
            "tokens": tokens,
        }));
    }
    println!("{}", lines.join("\n"));
    Ok(())
}

async fn chats(json: bool, limit: usize) -> Result<()> {
    if !(1..=100).contains(&limit) {
        bail!("limit must be between 1 and 100");
    }
    let threads = Client::new(load_jar()?)?.list_threads().await?;
    let shown: Vec<_> = threads.into_iter().take(limit).collect();
    if json {
        let items: Vec<Value> = shown
            .iter()
            .map(|thread| {
                json!({
                    "key": thread.thread_key,
                    "title": thread.title,
                    "kind": if thread.is_group { "group" } else { "direct" },
                    "last_activity_ms": thread.last_activity_ms,
                    "sync_group": thread.sync_group,
                })
            })
            .collect();
        return print_json(&Value::Array(items));
    }
    let mut lines = vec![format!("{} chats", shown.len())];
    for thread in &shown {
        let kind = if thread.is_group { "group" } else { "direct" };
        let title = thread.title.as_deref().unwrap_or("(untitled)");
        let when = thread
            .last_activity_ms
            .map(format_ms)
            .unwrap_or_else(|| "-".to_string());
        lines.push(format!(
            "- {title} [{kind}] {when} key={}",
            thread.thread_key
        ));
    }
    println!("{}", lines.join("\n"));
    Ok(())
}

async fn read(json: bool, chat_id: String, limit: u32, backend: Backend) -> Result<()> {
    check_chat_id(&chat_id)?;
    if !(1..=100).contains(&limit) {
        bail!("limit must be between 1 and 100");
    }
    let jar = load_jar()?;
    if backend == Backend::Browser {
        let messages = browser::read_messages(&chat_id, limit as usize, &jar).await?;
        if json {
            let items: Vec<Value> = messages
                .iter()
                .map(|message| json!({"id": message.id, "text": message.text}))
                .collect();
            return print_json(&Value::Array(items));
        }
        println!(
            "{} loaded messages, newest first; history may be incomplete",
            messages.len()
        );
        for message in &messages {
            println!("- {} (id {})", clip(&message.text, 500), message.id);
        }
        return Ok(());
    }

    let messages = Client::new(jar)?
        .read_messages(&chat_id, limit as usize, 1)
        .await?;
    if messages.is_empty() {
        bail!(
            "no messages returned for {chat_id}; it may be an end-to-end encrypted chat, \
             which this backend cannot read (use --backend browser)"
        );
    }
    if json {
        let items: Vec<Value> = messages
            .iter()
            .map(|message| {
                json!({
                    "id": message.message_id,
                    "ts_ms": message.ts_ms,
                    "sender": message.sender_id,
                    "text": message.text,
                })
            })
            .collect();
        return print_json(&Value::Array(items));
    }
    println!("{} messages", messages.len());
    for message in &messages {
        let sender = message.sender_id.as_deref().unwrap_or("?");
        let text = message.text.as_deref().unwrap_or("(no text)");
        println!("- {} {sender}: {text}", format_ms(message.ts_ms));
    }
    Ok(())
}

async fn send(
    json: bool,
    chat_id: String,
    text: String,
    backend: Backend,
    yes: bool,
) -> Result<()> {
    check_chat_id(&chat_id)?;
    let text = if text == "-" {
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .context("reading the message text from stdin")?;
        body
    } else {
        text
    };
    let text = text.trim();
    if text.is_empty() {
        bail!("message text is empty");
    }
    if !yes {
        confirm(&chat_id, text, backend)?;
    }

    let jar = load_jar()?;
    let (id, confirmed) = match backend {
        Backend::Browser => {
            let message = browser::send_message(&chat_id, text, &jar).await?;
            (Some(message.id), true)
        }
        Backend::Gateway => {
            let id = Client::new(jar)?.send_text(&chat_id, text, 1).await?;
            let confirmed = id.is_some();
            (id, confirmed)
        }
    };

    if json {
        return print_json(&json!({
            "chat_id": chat_id,
            "backend": backend.label(),
            "message_id": id,
            "confirmed": confirmed,
        }));
    }
    match (id, confirmed) {
        (Some(id), _) => println!("sent message {id} to {chat_id}"),
        (None, _) => println!(
            "sent to {chat_id}; reading the thread back did not confirm delivery. \
             If this is an encrypted chat, retry with --backend browser"
        ),
    }
    Ok(())
}

async fn react(json: bool, chat_id: String, message_id: String, emoji: String) -> Result<()> {
    check_chat_id(&chat_id)?;
    let message_id = message_id.trim();
    if message_id.is_empty() {
        bail!("message_id must not be empty");
    }
    let emoji = emoji.trim();
    if emoji.is_empty() || emoji.chars().count() > 8 {
        bail!("emoji must be one emoji");
    }
    let accepted = Client::new(load_jar()?)?
        .send_reaction(&chat_id, message_id, emoji, 1)
        .await?;
    if json {
        return print_json(&json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "emoji": emoji,
            "accepted": accepted,
        }));
    }
    if accepted {
        println!("reacted {emoji} to {message_id} in {chat_id}");
    } else {
        println!("sent the reaction for {message_id} to {chat_id}; the gateway did not confirm it");
    }
    Ok(())
}

async fn attach(
    json: bool,
    chat_id: String,
    path: PathBuf,
    caption: Option<String>,
    yes: bool,
) -> Result<()> {
    check_chat_id(&chat_id)?;
    if !path.is_file() {
        bail!("{} is not a file", path.display());
    }
    if !yes {
        confirm_attachment(&chat_id, &path, caption.as_deref())?;
    }
    let caption = caption.unwrap_or_default();
    let id = Client::new(load_jar()?)?
        .send_attachment(&chat_id, &path, &caption, 1)
        .await?;
    if json {
        return print_json(&json!({
            "chat_id": chat_id,
            "file": path.display().to_string(),
            "message_id": id,
            "confirmed": id.is_some(),
        }));
    }
    match id {
        Some(id) => println!("sent {} to {chat_id} (message {id})", path.display()),
        None => println!(
            "uploaded {} but the message did not appear in {chat_id}; try again",
            path.display()
        ),
    }
    Ok(())
}

fn confirm(chat_id: &str, text: &str, backend: Backend) -> Result<()> {
    ask(&format!(
        "send to {chat_id} via {}: \"{}\"",
        backend.label(),
        clip(text, 60)
    ))
}

fn confirm_attachment(chat_id: &str, path: &Path, caption: Option<&str>) -> Result<()> {
    let caption = match caption.filter(|caption| !caption.trim().is_empty()) {
        Some(caption) => format!(" with \"{}\"", clip(caption, 60)),
        None => String::new(),
    };
    ask(&format!("send {} to {chat_id}{caption}", path.display()))
}

fn ask(question: &str) -> Result<()> {
    if !io::stdin().is_terminal() {
        bail!("refusing to send without --yes when stdin is not a terminal");
    }
    eprint!("{question} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        bail!("cancelled");
    }
    Ok(())
}

fn load_jar() -> Result<CookieJar> {
    let path = auth::cookies_path()?;
    if !path.exists() {
        bail!(
            "no cookie file at {}; run `messenger login`",
            path.display()
        );
    }
    let jar = CookieJar::load(&path)?;
    if !jar.missing_required().is_empty() {
        bail!("required cookies are missing; run `messenger login`");
    }
    Ok(jar)
}

fn check_chat_id(chat_id: &str) -> Result<()> {
    if chat_id.is_empty() || !chat_id.bytes().all(|b| b.is_ascii_digit()) || chat_id.len() > 19 {
        bail!("chat_id must be a numeric thread ID");
    }
    Ok(())
}

fn format_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
