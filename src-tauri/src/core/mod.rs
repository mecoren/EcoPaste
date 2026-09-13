pub mod error;
pub mod metrics;
pub mod paths;
pub mod prevent_default;
#[cfg(target_os = "windows")]
pub mod windows_args;

pub use error::{AppError, Result};
