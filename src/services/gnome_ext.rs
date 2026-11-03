// Ayuz - Unofficial Control Center for Asus Laptops
// Copyright (C) 2026 Guido Philipp
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see https://www.gnu.org/licenses/.

//! Bundled GNOME Shell extension that provides the OLED care options GNOME has no
//! built-in interface for (top bar pixel shift, auto-hide, transparency, dimming of
//! inactive windows).
//!
//! The extension sources are embedded in the binary, copied to
//! `~/.local/share/gnome-shell/extensions/<UUID>/` on first use and driven through
//! its GSettings schema. On Wayland GNOME Shell only picks up a newly installed
//! extension after the next login.

use std::path::PathBuf;

use rust_i18n::t;

use super::commands::{run_command_blocking, run_command_stdout};

pub const UUID: &str = "ayuz@traciges.github.io";
const SCHEMA: &str = "org.gnome.shell.extensions.ayuz";

const FILES: [(&str, &str); 4] = [
    (
        "metadata.json",
        include_str!("../../packaging/gnome-extension/ayuz@traciges.github.io/metadata.json"),
    ),
    (
        "extension.js",
        include_str!("../../packaging/gnome-extension/ayuz@traciges.github.io/extension.js"),
    ),
    (
        "stylesheet.css",
        include_str!("../../packaging/gnome-extension/ayuz@traciges.github.io/stylesheet.css"),
    ),
    (
        "schemas/org.gnome.shell.extensions.ayuz.gschema.xml",
        include_str!(
            "../../packaging/gnome-extension/ayuz@traciges.github.io/schemas/org.gnome.shell.extensions.ayuz.gschema.xml"
        ),
    ),
];

fn extension_dir() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.data_dir().join("gnome-shell/extensions").join(UUID))
}

fn schema_dir() -> Result<String, String> {
    extension_dir()
        .map(|d| d.join("schemas").to_string_lossy().into_owned())
        .ok_or_else(|| t!("error_config_dir").to_string())
}

/// Copies the extension into the user's extension directory (only files whose content
/// changed), compiles its schema and enables it. Safe to call repeatedly.
pub async fn ensure_installed() -> Result<(), String> {
    let dir = extension_dir().ok_or_else(|| t!("error_config_dir").to_string())?;

    let changed = tokio::task::spawn_blocking({
        let dir = dir.clone();
        move || -> Result<bool, String> {
            let mut changed = false;
            for (name, content) in FILES {
                let path = dir.join(name);
                if std::fs::read_to_string(&path).ok().as_deref() == Some(content) {
                    continue;
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, content).map_err(|e| e.to_string())?;
                changed = true;
            }
            let compiled = dir.join("schemas/gschemas.compiled").exists();
            Ok(changed || !compiled)
        }
    })
    .await
    .map_err(|e| t!("error_spawn_blocking", error = e.to_string()).to_string())??;

    if changed {
        let schemas = schema_dir()?;
        run_command_blocking("glib-compile-schemas", &[&schemas]).await?;
    }

    enable().await
}

/// Enables the extension. Falls back to editing `enabled-extensions` directly when the
/// running shell does not know the extension yet (fresh install on Wayland).
async fn enable() -> Result<(), String> {
    if run_command_blocking("gnome-extensions", &["enable", UUID])
        .await
        .is_ok()
    {
        return Ok(());
    }
    let current = run_command_stdout("gsettings", &["get", "org.gnome.shell", "enabled-extensions"])
        .await?;
    let mut list = parse_strv(&current);
    if list.iter().any(|u| u == UUID) {
        return Ok(());
    }
    list.push(UUID.to_string());
    run_command_blocking(
        "gsettings",
        &["set", "org.gnome.shell", "enabled-extensions", &format_strv(&list)],
    )
    .await
}

/// `true` once GNOME Shell has actually loaded the extension (after the next login
/// on Wayland).
pub async fn is_active() -> bool {
    run_command_stdout("gnome-extensions", &["info", UUID])
        .await
        .map(|out| {
            out.lines().any(|l| {
                let l = l.trim();
                l.starts_with("State:") && (l.contains("ACTIVE") || l.contains("ENABLED"))
            })
        })
        .unwrap_or(false)
}

async fn set(key: &str, value: &str) -> Result<(), String> {
    ensure_installed().await?;
    let schemas = schema_dir()?;
    run_command_blocking(
        "gsettings",
        &["--schemadir", &schemas, "set", SCHEMA, key, value],
    )
    .await
}

pub async fn set_bool(key: &str, value: bool) -> Result<(), String> {
    set(key, if value { "true" } else { "false" }).await
}

pub async fn set_uint(key: &str, value: u32) -> Result<(), String> {
    set(key, &value.to_string()).await
}

/// Parses a GVariant string array as printed by `gsettings get`, e.g. `['a', 'b']` or `@as []`.
fn parse_strv(s: &str) -> Vec<String> {
    let s = s.trim().trim_start_matches("@as").trim();
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn format_strv(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|i| format!("'{i}'")).collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gsettings_string_arrays() {
        assert!(parse_strv("@as []").is_empty());
        assert_eq!(
            parse_strv("['a@b.org', 'c@d.io']\n"),
            vec!["a@b.org".to_string(), "c@d.io".to_string()]
        );
    }

    #[test]
    fn round_trips_string_arrays() {
        let list = vec!["x@y".to_string(), UUID.to_string()];
        assert_eq!(parse_strv(&format_strv(&list)), list);
    }

    #[test]
    fn bundled_schema_matches_constants() {
        let (_, xml) = FILES[3];
        assert!(FILES[3].0.ends_with(&format!("{SCHEMA}.gschema.xml")));
        assert!(xml.contains(&format!("id=\"{SCHEMA}\"")));
        let (_, meta) = FILES[0];
        assert!(meta.contains(UUID) && meta.contains(SCHEMA));
    }
}
