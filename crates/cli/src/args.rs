//! The command line: what `search` accepts and how it dispatches.

/// One parsed command. The absence of a subcommand means "serve MCP over
/// stdio", which is what a harness spawns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Serve MCP over stdio (the default, and what a harness invokes).
    Stdio { config: Option<String> },
    /// Serve the JSON API and MCP over HTTP.
    Serve {
        config: Option<String>,
        addr: Option<String>,
        data_dir: Option<String>,
    },
    /// Print the version.
    Version,
    /// Print usage.
    Help,
}

/// Parse arguments. An unknown first token that is not a flag is an error,
/// so a typo fails loudly rather than silently serving.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    let first = args.peek().cloned();
    match first.as_deref() {
        Some("serve") => {
            let _ = args.next();
            let flags = parse_serve_flags(args)?;
            Ok(Command::Serve {
                config: flags.config,
                addr: flags.addr,
                data_dir: flags.data_dir,
            })
        }
        Some("stdio") => {
            let _ = args.next();
            Ok(Command::Stdio {
                config: parse_config_flag(args)?,
            })
        }
        Some("version") | Some("--version") | Some("-version") => Ok(Command::Version),
        Some("help") | Some("--help") | Some("-h") => Ok(Command::Help),
        // Flags with no subcommand: default to stdio so harness configs that
        // pass `-config` still work.
        Some(flag) if flag.starts_with('-') => Ok(Command::Stdio {
            config: parse_config_flag(args)?,
        }),
        Some(other) => Err(format!("unknown command {other:?}")),
        None => Ok(Command::Stdio { config: None }),
    }
}

/// The usage text.
pub fn usage() -> &'static str {
    "search — self-hosted, agent-first web search

Usage:
  search                 serve MCP over stdio (what an agent spawns)
  search stdio [flags]   the same, explicit
  search serve [flags]   serve the JSON API and MCP over HTTP
  search version         print the version

Serve flags:
  -config PATH   JSON config (default $SEARCH_CONFIG or ~/.config/search/search.json)
  -addr ADDR     listen address (default 127.0.0.1:8642)
  -data DIR      data directory for the index"
}

fn parse_config_flag<I: IntoIterator<Item = String>>(args: I) -> Result<Option<String>, String> {
    let mut config = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-config" | "--config" => {
                config = Some(args.next().ok_or("missing value for -config")?);
            }
            other => return Err(format!("unknown flag {other:?}")),
        }
    }
    Ok(config)
}

fn parse_serve_flags<I: IntoIterator<Item = String>>(args: I) -> Result<ServeFlags, String> {
    let mut flags = ServeFlags::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-config" | "--config" => {
                flags.config = Some(args.next().ok_or("missing value for -config")?)
            }
            "-addr" | "--addr" => flags.addr = Some(args.next().ok_or("missing value for -addr")?),
            "-data" | "--data" => {
                flags.data_dir = Some(args.next().ok_or("missing value for -data")?)
            }
            other => return Err(format!("unknown flag {other:?}")),
        }
    }
    Ok(flags)
}

/// The serve-only flags, named so the parse signature stays readable.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ServeFlags {
    pub config: Option<String>,
    pub addr: Option<String>,
    pub data_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_stdio() {
        assert_eq!(parse(args(&[])).unwrap(), Command::Stdio { config: None });
    }

    #[test]
    fn serve_reads_its_flags() {
        assert_eq!(
            parse(args(&["serve", "-addr", "0.0.0.0:1", "-data", "/tmp/x"])).unwrap(),
            Command::Serve {
                config: None,
                addr: Some("0.0.0.0:1".into()),
                data_dir: Some("/tmp/x".into()),
            }
        );
    }

    #[test]
    fn a_lone_flag_defaults_to_stdio() {
        assert_eq!(
            parse(args(&["-config", "/etc/search.json"])).unwrap(),
            Command::Stdio {
                config: Some("/etc/search.json".into())
            }
        );
    }

    #[test]
    fn an_unknown_command_is_an_error() {
        assert!(parse(args(&["serve-me"])).is_err());
    }
}
