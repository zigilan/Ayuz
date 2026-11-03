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

//! Display mode control for GNOME via `org.gnome.Mutter.DisplayConfig`.

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

use super::refresh_rate::{RateInfo, RawMode, select};

/// `(connector, vendor, product, serial)`
type MonitorSpec = (String, String, String, String);
/// `(id, width, height, refresh, preferred_scale, supported_scales, properties)`
type RawModeTuple = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);
type RawMonitor = (MonitorSpec, Vec<RawModeTuple>, HashMap<String, OwnedValue>);
/// `(x, y, scale, transform, primary, monitors, properties)`
type RawLogical = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<MonitorSpec>,
    HashMap<String, OwnedValue>,
);
type State = (
    u32,
    Vec<RawMonitor>,
    Vec<RawLogical>,
    HashMap<String, OwnedValue>,
);
/// `(connector, mode_id, properties)`
type ApplyMonitor = (String, String, HashMap<String, OwnedValue>);
/// `(x, y, scale, transform, primary, monitors)`
type ApplyLogical = (i32, i32, f64, u32, bool, Vec<ApplyMonitor>);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig"
)]
trait DisplayConfig {
    fn get_current_state(&self) -> zbus::Result<State>;

    fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<ApplyLogical>,
        properties: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;
}

/// `ApplyMonitorsConfig` method: apply for this session only.
const METHOD_TEMPORARY: u32 = 1;

struct Monitor {
    connector: String,
    current_mode: Option<String>,
    modes: Vec<RawMode>,
}

fn flag(props: &HashMap<String, OwnedValue>, key: &str) -> bool {
    matches!(props.get(key).map(|v| &**v), Some(Value::Bool(true)))
}

fn monitors(raw: &[RawMonitor]) -> Vec<Monitor> {
    raw.iter()
        .map(|(spec, modes, _)| Monitor {
            connector: spec.0.clone(),
            current_mode: modes
                .iter()
                .find(|m| flag(&m.6, "is-current"))
                .map(|m| m.0.clone()),
            modes: modes
                .iter()
                .map(|m| RawMode {
                    id: m.0.clone(),
                    size: (i64::from(m.1), i64::from(m.2)),
                    hz: m.3,
                })
                .collect(),
        })
        .collect()
}

fn rates(monitors: &[Monitor], connector: &str) -> Option<RateInfo> {
    let m = monitors.iter().find(|m| m.connector == connector)?;
    select(&m.modes, m.current_mode.as_deref()?)
}

/// Rebuilds the current logical-monitor layout, changing only `target` to `mode_id`.
fn build_apply(
    logicals: &[RawLogical],
    monitors: &[Monitor],
    target: &str,
    mode_id: &str,
) -> Result<Vec<ApplyLogical>, String> {
    logicals
        .iter()
        .map(|(x, y, scale, transform, primary, specs, _)| {
            let members = specs
                .iter()
                .map(|spec| {
                    let connector = &spec.0;
                    let mode = if connector == target {
                        mode_id.to_string()
                    } else {
                        monitors
                            .iter()
                            .find(|m| &m.connector == connector)
                            .and_then(|m| m.current_mode.clone())
                            .ok_or_else(|| format!("no current mode for {connector}"))?
                    };
                    Ok((connector.clone(), mode, HashMap::new()))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok((*x, *y, *scale, *transform, *primary, members))
        })
        .collect()
}

async fn proxy() -> Result<DisplayConfigProxy<'static>, String> {
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    DisplayConfigProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())
}

/// Refresh rates of `connector` at its current resolution.
pub async fn read_rates(connector: &str) -> Result<Option<RateInfo>, String> {
    let (_, raw, _, _) = proxy()
        .await?
        .get_current_state()
        .await
        .map_err(|e| e.to_string())?;
    Ok(rates(&monitors(&raw), connector))
}

/// Switches `connector` to `mode_id`, leaving every other monitor and the layout untouched.
pub async fn set_mode(connector: &str, mode_id: &str) -> Result<(), String> {
    let proxy = proxy().await?;
    let (serial, raw, logicals, props) = proxy
        .get_current_state()
        .await
        .map_err(|e| e.to_string())?;
    let apply = build_apply(&logicals, &monitors(&raw), connector, mode_id)?;

    let mut out_props: HashMap<String, OwnedValue> = HashMap::new();
    if let Some(Value::U32(layout)) = props.get("layout-mode").map(|v| &**v) {
        if let Ok(v) = OwnedValue::try_from(Value::U32(*layout)) {
            out_props.insert("layout-mode".to_string(), v);
        }
    }

    proxy
        .apply_monitors_config(serial, METHOD_TEMPORARY, apply, out_props)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str, hz: f64, current: bool) -> RawModeTuple {
        let mut props = HashMap::new();
        if current {
            props.insert(
                "is-current".to_string(),
                OwnedValue::try_from(Value::Bool(true)).unwrap(),
            );
        }
        (id.to_string(), 2880, 1800, hz, 2.0, vec![1.0, 2.0], props)
    }

    fn spec(c: &str) -> MonitorSpec {
        (c.into(), "SDC".into(), "x".into(), "0".into())
    }

    fn sample() -> (Vec<RawMonitor>, Vec<RawLogical>) {
        let raw = vec![
            (
                spec("eDP-1"),
                vec![
                    mode("2880x1800@120.000", 120.0, false),
                    mode("2880x1800@60.000", 60.0, true),
                ],
                HashMap::new(),
            ),
            (
                spec("HDMI-1"),
                vec![mode("1920x1080@60.000", 60.0, true)],
                HashMap::new(),
            ),
        ];
        let logicals = vec![
            (0, 0, 2.0, 0, true, vec![spec("eDP-1")], HashMap::new()),
            (1440, 0, 1.0, 0, false, vec![spec("HDMI-1")], HashMap::new()),
        ];
        (raw, logicals)
    }

    #[test]
    fn reads_rates_of_requested_connector() {
        let (raw, _) = sample();
        let info = rates(&monitors(&raw), "eDP-1").unwrap();
        assert_eq!(info.modes.len(), 2);
        assert_eq!(info.modes[info.current].id, "2880x1800@60.000");
        assert!(rates(&monitors(&raw), "DP-9").is_none());
    }

    #[test]
    fn apply_changes_only_target_and_keeps_layout() {
        let (raw, logicals) = sample();
        let cfg = build_apply(&logicals, &monitors(&raw), "eDP-1", "2880x1800@120.000").unwrap();
        assert_eq!(cfg.len(), 2);
        assert_eq!(cfg[0].5[0].1, "2880x1800@120.000");
        assert_eq!(cfg[1].5[0].1, "1920x1080@60.000");
        assert_eq!((cfg[1].0, cfg[1].1, cfg[1].2), (1440, 0, 1.0));
        assert!(cfg[0].4 && !cfg[1].4);
    }

    #[test]
    fn missing_current_mode_is_an_error() {
        let (mut raw, logicals) = sample();
        raw[1].1[0].6.clear();
        assert!(build_apply(&logicals, &monitors(&raw), "eDP-1", "x").is_err());
    }
}
