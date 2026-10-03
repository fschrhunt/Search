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
        Command::Search {
            query,
            limit,
            json,
            providers,
            config,
        } => search_command(query, limit, json, providers, config).await,
        Command::Fetch {
            urls,
            query,
            max_characters,
            json,
            config,
        } => fetch_command(urls, query, max_characters, json, config).await,
        Command::Index {
            query,
            limit,
            json,
            config,
        } => index_command(query, limit, json, config).await,
        Command::Refresh { config } => refresh_command(config).await,
    }
}

async fn search_command(
    query: String,
    limit: usize,
    json: bool,
    providers: Vec<String>,
    config_path: Option<String>,
) -> i32 {
    let service = match build_service(config_path, None, None) {
        Ok(service) => service,
        Err(error) => return report_error(error),
    };
    let mut request = search_core::discovery::Query {
        text: query,
        ..Default::default()
    };
    request.limit = limit;
    if !providers.is_empty() {
        request.providers = providers;
    }
    let response = service.search(request).await;
    if json {
        crate::render::json(&response)
    } else {
        crate::render::search(&response)
    }
}

async fn fetch_command(
    urls: Vec<String>,
    query: Option<String>,
    max_characters: Option<usize>,
    json: bool,
    config_path: Option<String>,
) -> i32 {
    let service = match build_service(config_path, None, None) {
        Ok(service) => service,
        Err(error) => return report_error(error),
    };
    let mut results = service.fetch(&urls).await;
    for result in &mut results {
        if let Some(focus) = query.as_deref() {
            result.text =
                search_core::text::select(&result.text, focus, max_characters.unwrap_or(4000))
                    .into_iter()
                    .map(|passage| passage.text)
                    .collect::<Vec<_>>()
                    .join("\n\n");
        } else if let Some(max) = max_characters {
            result.text = result.text.chars().take(max).collect();
        }
    }
    if json {
        crate::render::json(&results)
    } else {
        crate::render::fetch(&results)
    }
}

async fn index_command(
    query: String,
    limit: usize,
    json: bool,
    config_path: Option<String>,
) -> i32 {
    let service = match build_service(config_path, None, None) {
        Ok(service) => service,
        Err(error) => return report_error(error),
    };
    match service.index_search(&query, limit) {
        Ok(results) => {
            if json {
                crate::render::json(&results)
            } else {
                crate::render::hits(&results)
            }
        }
        Err(error) => report_error(error.to_string()),
    }
}

async fn refresh_command(config_path: Option<String>) -> i32 {
    let service = match build_service(config_path, None, None) {
        Ok(service) => service,
        Err(error) => return report_error(error),
    };
    let refreshed = service.refresh_seeded().await;
    println!("Refreshed {refreshed} document(s)");
    0
}

fn report_error(error: String) -> i32 {
    eprintln!("search: {error}");
    1
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
