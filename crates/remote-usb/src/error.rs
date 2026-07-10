use thiserror::Error;

/// Application errors with actionable messages.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("privilege required: {0} (try running with sudo)")]
    PrivilegeRequired(String),

    #[error("kernel module '{module}' is not loaded and could not be loaded: {detail}")]
    ModuleLoad { module: String, detail: String },

    #[error("usbip tool not found: {0}. Install linux-tools / usbip package for your distro")]
    ToolNotFound(String),

    #[error("usbip command failed: {cmd}\n{stderr}")]
    UsbipFailed { cmd: String, stderr: String },

    #[error("device not found matching '{selector}'")]
    DeviceNotFound { selector: String },

    #[error(
        "ambiguous device selector '{selector}': matches {count} devices; use a busid instead"
    )]
    AmbiguousSelector { selector: String, count: usize },

    #[error("failed to parse usbip output: {0}")]
    Parse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
