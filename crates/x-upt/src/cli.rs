//! Command-line argument and environment variable parser for `x-upt`.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Default control-plane listen address when neither CLI flag nor env var is set.
pub const DEFAULT_ADMIN_LISTEN: &str = "0.0.0.0:18768";
/// Default data directory when neither CLI flag nor env var is set.
pub const DEFAULT_DATA_DIR: &str = "./data";

/// Parsed subcommand for `x-upt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Starts the control plane and restores enabled upstream fronts.
    Serve(RunConfig),
    /// Validates `SQLite` migrations, snapshot loading, and protocol contracts without binding ports.
    Check(RunConfig),
    /// Prints binary version information.
    Version,
    /// Prints CLI usage text.
    Help,
}

/// Runtime options resolved from CLI flags and `X_UPT_*` environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfig {
    /// Control-plane bind `SocketAddr`.
    pub admin_addr: SocketAddr,
    /// Raw control-plane listen string.
    pub admin_listen: String,
    /// Persistent data directory (`x-upt.db`, `images/`, `logs/`).
    pub data_dir: PathBuf,
    /// Optional management bearer/login token.
    pub admin_token: Option<String>,
}

/// Parses `std::env::args()` and `X_UPT_*` environment variables into a [`Command`].
///
/// # Errors
/// Returns a descriptive error string on unknown flags or invalid socket addresses.
pub fn parse_cli<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let _bin = iter.next();
    let vec_args: Vec<String> = iter.collect();

    if vec_args
        .iter()
        .any(|a| a == "-h" || a == "--help" || a == "help")
    {
        return Ok(Command::Help);
    }
    if vec_args
        .iter()
        .any(|a| a == "-V" || a == "--version" || a == "version")
    {
        return Ok(Command::Version);
    }

    let mut mode = "serve";
    let mut idx = 0usize;
    if let Some(first) = vec_args.first().map(String::as_str)
        && !first.starts_with('-')
    {
        match first {
            "serve" => {
                mode = "serve";
                idx = 1;
            }
            "check" => {
                mode = "check";
                idx = 1;
            }
            other => return Err(format!("unknown subcommand: {other}")),
        }
    }

    let mut admin_listen = std::env::var("X_UPT_ADMIN_LISTEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ADMIN_LISTEN.to_string());
    let mut data_dir = std::env::var("X_UPT_DATA_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string());
    let mut admin_token = std::env::var("X_UPT_ADMIN_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    while let Some(arg) = vec_args.get(idx) {
        match arg.as_str() {
            "--admin-listen" => {
                let val = vec_args
                    .get(idx + 1)
                    .ok_or_else(|| "missing value for --admin-listen".to_string())?;
                admin_listen.clone_from(val);
                idx += 2;
            }
            "--data-dir" => {
                let val = vec_args
                    .get(idx + 1)
                    .ok_or_else(|| "missing value for --data-dir".to_string())?;
                data_dir.clone_from(val);
                idx += 2;
            }
            "--admin-token" => {
                let val = vec_args
                    .get(idx + 1)
                    .ok_or_else(|| "missing value for --admin-token".to_string())?;
                let trimmed = val.trim().to_string();
                admin_token = (!trimmed.is_empty()).then_some(trimmed);
                idx += 2;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }

    let admin_addr: SocketAddr = admin_listen
        .parse()
        .map_err(|e| format!("invalid --admin-listen '{admin_listen}': {e}"))?;

    let cfg = RunConfig {
        admin_addr,
        admin_listen,
        data_dir: PathBuf::from(data_dir),
        admin_token,
    };
    if mode == "check" {
        Ok(Command::Check(cfg))
    } else {
        Ok(Command::Serve(cfg))
    }
}

/// Usage help text for `x-upt --help`.
#[must_use]
pub fn help_text() -> &'static str {
    "x-upt - Stateful multi-protocol mock upstream server\n\n\
     USAGE:\n  \
       x-upt [serve|check|version] [OPTIONS]\n\n\
     OPTIONS:\n  \
       --admin-listen <ADDR>  Control-plane listen address (env: X_UPT_ADMIN_LISTEN, default: 0.0.0.0:18768)\n  \
       --data-dir <DIR>       SQLite & asset directory (env: X_UPT_DATA_DIR, default: ./data)\n  \
       --admin-token <TOKEN>  Optional management token (env: X_UPT_ADMIN_TOKEN)\n  \
       -h, --help             Show help\n  \
       -V, --version          Show version\n"
}
