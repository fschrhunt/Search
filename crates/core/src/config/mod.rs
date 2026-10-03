//! The service's settings: where it listens, where its data lives, how long a
//! query may run, and which providers are enabled.
//!
//! Every field has a safe default; a config file only overrides what it names.
//! The one secret is the token, and it may come from the file or, preferably,
//! the named environment variable, so a config file can be shared without
//! leaking the credential.

mod defaults;
mod load;
mod settings;

pub use settings::LogLevel;
pub use settings::{
    Config, EngineSettings, FetchSettings, IndexSettings, SearchSettings, DEFAULT_ADDR,
    DEFAULT_TOKEN_ENV,
};

/// Load and validate configuration from `path`, falling back to `SEARCH_CONFIG`
/// and then the standard location. Environment overrides are applied last.
pub use load::load;
