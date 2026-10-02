//! Dispatch and the serve loop.

use std::sync::Arc;

use search_core::config;
use search_core::search::Service;

use crate::{args::Command, http, stdio};

/// Run one parsed command. Returns the process exit code.
pub async fn execute(command: Command) -> i32 {
    match command {
        Command::Version => {
            println!("{}", search_core::VERSION);
            0
        }
        Command::Help => {
            println!("{}", crate::args::usage());
            0
        }
        Command::Stdio { config } => stdio::serve(config).await,
        Command::Serve {
            config,
            addr,
            data_dir,
        } => http::serve(config, addr, data_dir).await,
    }
}

/// Load configuration and build the service, applying serve overrides before
/// validating the effective settings.
pub(super) fn build_service(
    config_path: Option<String>,
    addr: Option<String>,
    data_dir: Option<String>,
) -> Result<Arc<Service>, String> {
    let path = config_path.map(std::path::PathBuf::from);
    let mut settings = config::load(path).map_err(|e| e.message().to_string())?;
    if let Some(addr) = addr {
        settings.addr = addr;
    }
    if let Some(dir) = data_dir {
        settings.data_dir = std::path::PathBuf::from(dir);
    }
    // Overrides bypassed validation at load time, so re-check the effective
    // settings before binding.
    settings.validate().map_err(|e| e.message().to_string())?;
    let service = Service::open(settings).map_err(|e| e.to_string())?;
    Ok(Arc::new(service))
}
