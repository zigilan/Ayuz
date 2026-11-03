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

//! ICC profile assignment through colord, used on GNOME where Mutter (49+) applies the
//! default colord profile of each display.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use super::commands::run_command_blocking;

#[zbus::proxy(
    interface = "org.freedesktop.ColorManager",
    default_service = "org.freedesktop.ColorManager",
    default_path = "/org/freedesktop/ColorManager"
)]
trait ColorManager {
    fn get_devices_by_kind(&self, kind: &str) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn find_profile_by_filename(&self, filename: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.ColorManager.Device",
    default_service = "org.freedesktop.ColorManager"
)]
trait ColorDevice {
    fn add_profile(&self, relation: &str, object_path: &ObjectPath<'_>) -> zbus::Result<()>;
    fn remove_profile(&self, object_path: &ObjectPath<'_>) -> zbus::Result<()>;
    fn make_profile_default(&self, object_path: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, String>>;
}

/// Metadata key under which the compositor stores the connector name of a display.
const XRANDR_NAME: &str = "XRANDR_name";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Picks the device whose connector matches; on a single-display laptop the only
/// display device is used even if it carries no connector metadata.
fn pick_device(candidates: &[(OwnedObjectPath, Option<String>)], connector: &str) -> Option<OwnedObjectPath> {
    candidates
        .iter()
        .find(|(_, name)| name.as_deref() == Some(connector))
        .or_else(|| (candidates.len() == 1).then(|| &candidates[0]))
        .map(|(path, _)| path.clone())
}

async fn display_device<'a>(
    conn: &'a zbus::Connection,
    connector: &str,
) -> Result<ColorDeviceProxy<'a>, String> {
    let manager = ColorManagerProxy::new(conn).await.map_err(err)?;
    let mut candidates = Vec::new();
    for path in manager.get_devices_by_kind("display").await.map_err(err)? {
        let device = ColorDeviceProxy::builder(conn)
            .path(path.clone())
            .map_err(err)?
            .build()
            .await
            .map_err(err)?;
        let name = device
            .metadata()
            .await
            .ok()
            .and_then(|m| m.get(XRANDR_NAME).cloned());
        candidates.push((path, name));
    }
    let path = pick_device(&candidates, connector)
        .ok_or_else(|| format!("colord: no display device for {connector}"))?;
    ColorDeviceProxy::builder(conn)
        .path(path)
        .map_err(err)?
        .build()
        .await
        .map_err(err)
}

/// Where `colormgr import-profile` copies a profile to.
fn imported_path(file: &Path) -> Option<PathBuf> {
    let name = file.file_name()?;
    directories::BaseDirs::new().map(|b| b.data_dir().join("icc").join(name))
}

async fn find_profile(manager: &ColorManagerProxy<'_>, file: &Path) -> Option<OwnedObjectPath> {
    let target = imported_path(file)?;
    manager
        .find_profile_by_filename(&target.to_string_lossy())
        .await
        .ok()
}

/// Registers `file` with colord (once) and makes it the default profile of `connector`.
pub async fn apply_profile(connector: &str, file: &Path) -> Result<(), String> {
    let conn = zbus::Connection::system().await.map_err(err)?;
    let manager = ColorManagerProxy::new(&conn).await.map_err(err)?;

    let profile = match find_profile(&manager, file).await {
        Some(p) => p,
        None => {
            // colormgr hands the file to colord via an fd, which colord needs because it
            // cannot read the user's home directory itself.
            run_command_blocking("colormgr", &["import-profile", &file.to_string_lossy()]).await?;
            find_profile(&manager, file)
                .await
                .ok_or_else(|| format!("colord: profile {} not registered", file.display()))?
        }
    };

    let device = display_device(&conn, connector).await?;
    // Already assigned is fine; colord reports that as an error we can ignore.
    let _ = device.add_profile("hard", &profile).await;
    device.make_profile_default(&profile).await.map_err(err)
}

/// Removes the given profiles from `connector`, so the compositor falls back to the
/// panel's own (EDID) profile.
pub async fn remove_profiles(connector: &str, files: &[PathBuf]) -> Result<(), String> {
    let conn = zbus::Connection::system().await.map_err(err)?;
    let manager = ColorManagerProxy::new(&conn).await.map_err(err)?;
    let device = display_device(&conn, connector).await?;
    for file in files {
        if let Some(profile) = find_profile(&manager, file).await {
            let _ = device.remove_profile(&profile).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(s).unwrap()
    }

    #[test]
    fn picks_device_by_connector() {
        let c = vec![
            (p("/a"), Some("HDMI-1".to_string())),
            (p("/b"), Some("eDP-1".to_string())),
        ];
        assert_eq!(pick_device(&c, "eDP-1"), Some(p("/b")));
        assert_eq!(pick_device(&c, "DP-3"), None);
    }

    #[test]
    fn single_display_is_used_without_metadata() {
        let c = vec![(p("/only"), None)];
        assert_eq!(pick_device(&c, "eDP-1"), Some(p("/only")));
    }
}
