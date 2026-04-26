//! Bundle export/import metadata.
//!
//! Every bundle (sandbox VM tarball OR native tree tarball) that clawenv
//! produces carries a `clawenv-bundle.toml` at its archive root. The file
//! identifies the claw type, sandbox backend, source platform and the
//! producing clawenv version, so the import side can refuse incompatible
//! bundles up-front instead of discovering the mismatch mid-extract.
//!
//! Lifted from v1 `core/src/export/`. The schema and wrap/unwrap helpers
//! are unchanged so v1 and v2 bundles share the same on-wire format —
//! a v2 `clawcli import` should be able to consume a v1-produced bundle
//! once we wire the sandbox_type kebab-case translator (deferred to P3).

pub mod manifest;

pub use manifest::{BundleManifest, MANIFEST_FILENAME};
