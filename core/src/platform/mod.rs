mod detector;
pub mod network;
pub mod process;

pub use detector::{detect_platform, PlatformInfo, OsType, Arch};
