use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Native implementation using RustEmbed
#[derive(rust_embed::RustEmbed)]
#[folder = "src/icons"]
#[include = "**/*.svg"]
pub struct Assets;

impl Assets {
    /// Create a new Assets instance. The endpoint parameter is ignored for native builds.
    pub fn new(_endpoint: impl Into<SharedString>) -> Self {
        Self
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        // gpui-component requests paths like "icons/arrow-down.svg".
        // RustEmbed stores them without the "icons/" prefix, so strip it.
        let key = path.strip_prefix("icons/").unwrap_or(path);

        Self::get(key)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("could not find asset at path \"{}\"", path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = path.strip_prefix("icons/").unwrap_or(path);
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(prefix).then(|| format!("icons/{p}").into()))
            .collect())
    }
}
