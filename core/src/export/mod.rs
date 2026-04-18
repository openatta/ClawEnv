//! Bundle export/import metadata.
//!
//! Every bundle (sandbox VM tarball OR native tree tarball) that clawenv
//! produces carries a `clawenv-bundle.toml` at its archive root. The file
//! identifies the claw type, sandbox backend, source platform and the
//! producing clawenv version, so the import side can refuse incompatible
//! bundles up-front instead of discovering the mismatch mid-extract.

pub mod manifest;

pub use manifest::{BundleManifest, MANIFEST_FILENAME};
