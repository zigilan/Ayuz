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

//! Parsing of `kscreen-doctor -j` output into the refresh rates a panel offers
//! at its current resolution.

use serde_json::Value;

/// One selectable refresh rate and the `kscreen-doctor` mode id that applies it.
#[derive(Clone, Debug, PartialEq)]
pub struct RateMode {
    pub id: String,
    pub hz: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RateInfo {
    /// Distinct rates at the current resolution, fastest first.
    pub modes: Vec<RateMode>,
    /// Index into `modes` of the active rate.
    pub current: usize,
}

/// Whole-Hz key used to store and compare rates (59.94 and 60.0 are both 60).
pub fn hz_key(hz: f64) -> u32 {
    hz.round().max(0.0) as u32
}

/// Human readable rate, e.g. `120 Hz` or `59.94 Hz`.
pub fn label(hz: f64) -> String {
    if (hz - hz.round()).abs() < 0.05 {
        format!("{} Hz", hz.round() as u32)
    } else {
        format!("{hz:.2} Hz")
    }
}

fn id_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Extracts the refresh rates of `output` at its current resolution.
///
/// Returns `None` if the output is missing, disabled, or reports no usable modes.
pub fn parse(json: &str, output: &str) -> Option<RateInfo> {
    let root: Value = serde_json::from_str(json).ok()?;
    let out = root
        .get("outputs")?
        .as_array()?
        .iter()
        .find(|o| o.get("name").and_then(Value::as_str) == Some(output))?;
    if out.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let current_id = id_string(out.get("currentModeId")?)?;
    let raw: Vec<RawMode> = out
        .get("modes")?
        .as_array()?
        .iter()
        .filter_map(|m| {
            let s = m.get("size")?;
            Some(RawMode {
                id: id_string(m.get("id")?)?,
                size: (s.get("width")?.as_i64()?, s.get("height")?.as_i64()?),
                hz: m.get("refreshRate")?.as_f64()?,
            })
        })
        .collect();
    select(&raw, &current_id)
}

/// A display mode as reported by a compositor backend.
#[derive(Clone, Debug)]
pub struct RawMode {
    pub id: String,
    pub size: (i64, i64),
    pub hz: f64,
}

/// Reduces all modes of an output to the distinct refresh rates at the resolution of
/// the mode `current_id`. Returns `None` if `current_id` is not among `modes`.
pub fn select(modes: &[RawMode], current_id: &str) -> Option<RateInfo> {
    let current_size = modes.iter().find(|m| m.id == current_id)?.size;

    let mut candidates: Vec<RateMode> = modes
        .iter()
        .filter(|m| m.size == current_size)
        .map(|m| RateMode {
            id: m.id.clone(),
            hz: m.hz,
        })
        .collect();
    candidates.sort_by(|a, b| b.hz.total_cmp(&a.hz));
    let current_id = current_id.to_string();

    // Keep one mode per whole-Hz value, preferring the one that is currently active.
    let mut unique: Vec<RateMode> = Vec::new();
    for m in candidates {
        match unique.iter().position(|u| hz_key(u.hz) == hz_key(m.hz)) {
            Some(i) if m.id == current_id => unique[i] = m,
            Some(_) => {}
            None => unique.push(m),
        }
    }

    let current = unique.iter().position(|m| m.id == current_id)?;
    Some(RateInfo {
        modes: unique,
        current,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"outputs":[
        {"name":"HDMI-A-1","enabled":true,"currentModeId":"1","modes":[
            {"id":"1","refreshRate":60.0,"size":{"width":1920,"height":1080}}]},
        {"name":"eDP-1","enabled":true,"currentModeId":"3","modes":[
            {"id":"1","refreshRate":120.0,"size":{"width":2880,"height":1800}},
            {"id":"2","refreshRate":60.001,"size":{"width":2880,"height":1800}},
            {"id":"3","refreshRate":59.94,"size":{"width":2880,"height":1800}},
            {"id":"4","refreshRate":60.0,"size":{"width":1920,"height":1200}}]}]}"#;

    #[test]
    fn picks_rates_at_current_resolution_only() {
        let info = parse(SAMPLE, "eDP-1").unwrap();
        let ids: Vec<&str> = info.modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["1", "3"]);
    }

    #[test]
    fn prefers_active_mode_among_same_whole_hz() {
        let info = parse(SAMPLE, "eDP-1").unwrap();
        assert_eq!(info.modes[info.current].id, "3");
    }

    #[test]
    fn missing_or_disabled_output_is_none() {
        assert!(parse(SAMPLE, "DP-9").is_none());
        let off = r#"{"outputs":[{"name":"eDP-1","enabled":false,"currentModeId":"1","modes":[]}]}"#;
        assert!(parse(off, "eDP-1").is_none());
        assert!(parse("not json", "eDP-1").is_none());
    }

    #[test]
    fn numeric_ids_are_accepted() {
        let j = r#"{"outputs":[{"name":"eDP-1","currentModeId":2,"modes":[
            {"id":1,"refreshRate":120.0,"size":{"width":10,"height":10}},
            {"id":2,"refreshRate":60.0,"size":{"width":10,"height":10}}]}]}"#;
        let info = parse(j, "eDP-1").unwrap();
        assert_eq!(info.modes[info.current].id, "2");
    }

    #[test]
    fn labels() {
        assert_eq!(label(120.0), "120 Hz");
        assert_eq!(label(59.94), "59.94 Hz");
        assert_eq!(label(60.001), "60 Hz");
        assert_eq!(hz_key(59.94), 60);
    }
}
