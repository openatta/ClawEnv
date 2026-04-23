//! NativeOps — host-side runtime (Node.js, Git, clawenv home) management.

pub mod types;
pub mod ops;
pub mod default;

pub use ops::NativeOps;
pub use types::{
    Component, ComponentInfo, NativeDoctorIssue, NativeDoctorReport, NativeStatus, VersionSpec,
};
pub use default::DefaultNativeOps;
