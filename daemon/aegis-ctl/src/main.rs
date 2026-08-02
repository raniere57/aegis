use std::path::PathBuf;

use aegis_core::ipc::{encode_line, parse_line, Request, Response};
use aegis_core::paths::AegisPaths;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Parser, Debug)]
#[command(name = "aegis-ctl", version, about = "Control client for aegisd")]
struct Args {
    #[arg(long)]
    socket: Option<PathBuf>,

    #[arg(long)]
    dev: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Ping,
    Status,
    Metrics,
    Enable,
    Disable,
    UpdateLists,
    ReloadLists,
    ReloadConfig,
    Allowlist {
        #[command(subcommand)]
        action: AllowCmd,
    },
    Lists {
        #[command(subcommand)]
        action: ListsCmd,
    },
    Raw {
        method: String,
        #[arg(default_value = "{}")]
        params: String,
    },
}

#[derive(Subcommand, Debug)]
enum AllowCmd {
    List,
    Add { domain: String },
    Remove { domain: String },
}

#[derive(Subcommand, Debug)]
enum ListsCmd {
    List,
    Add { url: String },
    Remove { url: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let paths = if args.dev {
        AegisPaths::dev()
    } else if std::path::Path::new("/var/run/aegis.sock").exists() {
        AegisPaths::privileged()
    } else {
        AegisPaths::dev()
    };
    let socket = args.socket.unwrap_or(paths.socket);

    let (method, params) = match args.cmd {
        Cmd::Ping => ("ping", json!({})),
        Cmd::Status => ("status", json!({})),
        Cmd::Metrics => ("metrics", json!({})),
        Cmd::Enable => ("set_enabled", json!({"enabled": true})),
        Cmd::Disable => ("set_enabled", json!({"enabled": false})),
        Cmd::UpdateLists => ("update_lists", json!({})),
        Cmd::ReloadLists => ("reload_lists", json!({})),
        Cmd::ReloadConfig => ("reload_config", json!({})),
        Cmd::Allowlist { action } => match action {
            AllowCmd::List => ("allowlist.list", json!({})),
            AllowCmd::Add { domain } => ("allowlist.add", json!({"domain": domain})),
            AllowCmd::Remove { domain } => ("allowlist.remove", json!({"domain": domain})),
        },
        Cmd::Lists { action } => match action {
            ListsCmd::List => ("lists.list", json!({})),
            ListsCmd::Add { url } => ("lists.add_url", json!({"url": url})),
            ListsCmd::Remove { url } => ("lists.remove_url", json!({"url": url})),
        },
        Cmd::Raw { method, params } => {
            let v: Value = serde_json::from_str(&params).context("params json")?;
            let resp = rpc(&socket, &method, v).await?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
            return Ok(());
        }
    };

    let resp = rpc(&socket, method, params).await?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    if !resp.ok {
        bail!("command failed");
    }
    Ok(())
}

async fn rpc(socket: &PathBuf, method: &str, params: Value) -> Result<Response> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connect {}", socket.display()))?;
    let (reader, mut writer) = stream.into_split();
    let req = Request {
        id: uuid::Uuid::new_v4().to_string(),
        method: method.to_string(),
        params,
    };
    // Re-encode request as line
    let mut line = serde_json::to_string(&req)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    let mut lines = BufReader::new(reader).lines();
    let Some(resp_line) = lines.next_line().await? else {
        bail!("no response");
    };
    let _ = encode_line; // keep import used if needed
    let _ = parse_line;
    Ok(serde_json::from_str(&resp_line)?)
}
