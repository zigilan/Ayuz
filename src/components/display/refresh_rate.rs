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

use gtk4 as gtk;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;
use rust_i18n::t;

use super::helpers::DISPLAY_NAME;
use crate::services::commands::{
    is_gnome_desktop, is_kde_desktop, run_command_blocking, run_command_stdout,
};
use crate::services::config::AppConfig;
use crate::services::mutter_display;
use crate::services::refresh_rate::{self, RateInfo, RateMode};

pub struct RefreshRateModel {
    modes: Vec<RateMode>,
    selected: u32,
    /// Rate (whole Hz) from the active profile that should be applied; 0 keeps the current one.
    wanted_hz: u32,
    handler_connected: bool,
    rate_row: adw::ComboRow,
    /// KDE (`kscreen-doctor`) or GNOME (Mutter D-Bus).
    supported: bool,
}

#[derive(Debug)]
pub enum RefreshRateMsg {
    ChangeRate(u32),
    LoadProfile(u32),
}

#[derive(Debug)]
pub enum RefreshRateCommandOutput {
    ModesRead(Option<RateInfo>),
    RateApplied(u32),
    Error(String),
}

#[relm4::component(pub)]
impl Component for RefreshRateModel {
    type Init = ();
    type Input = RefreshRateMsg;
    type Output = String;
    type CommandOutput = RefreshRateCommandOutput;

    view! {
        adw::PreferencesGroup {
            set_title: &t!("refresh_rate_group_title"),
            set_description: Some(&t!("refresh_rate_group_desc")),

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                #[watch]
                set_visible: !model.supported,
                set_label: &t!("kde_or_gnome_required_warning"),
            },

            add = &model.rate_row.clone() -> adw::ComboRow {
                set_title: &t!("refresh_rate_title"),
                set_subtitle: &t!("refresh_rate_subtitle"),

                #[watch]
                set_sensitive: model.supported && model.modes.len() > 1,
                #[watch]
                set_selected: model.selected,
            },
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = AppConfig::load();
        let supported = is_kde_desktop() || is_gnome_desktop();

        let model = RefreshRateModel {
            modes: Vec::new(),
            selected: 0,
            wanted_hz: config.active_profile().display_refresh_rate,
            handler_connected: false,
            rate_row: adw::ComboRow::new(),
            supported,
        };
        let widgets = view_output!();

        if supported {
            sender.command(|out, shutdown| {
                shutdown
                    .register(async move {
                        let info = read_modes().await.unwrap_or_else(|e| {
                            tracing::warn!("refresh rate query: {e}");
                            None
                        });
                        out.emit(RefreshRateCommandOutput::ModesRead(info));
                    })
                    .drop_on_shutdown()
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: RefreshRateMsg, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            RefreshRateMsg::ChangeRate(idx) => {
                if idx == self.selected || idx as usize >= self.modes.len() {
                    return;
                }
                self.selected = idx;
                let mode = self.modes[idx as usize].clone();
                let hz = refresh_rate::hz_key(mode.hz);
                self.wanted_hz = hz;
                AppConfig::update(|c| c.active_profile_mut().display_refresh_rate = hz);
                apply_mode(mode, &sender);
            }
            RefreshRateMsg::LoadProfile(hz) => {
                self.wanted_hz = hz;
                if self.supported && hz > 0 {
                    self.apply_wanted(&sender);
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: RefreshRateCommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            RefreshRateCommandOutput::ModesRead(None) => {
                tracing::info!("{}", t!("refresh_rate_no_modes"));
            }
            RefreshRateCommandOutput::ModesRead(Some(info)) => {
                let labels: Vec<String> =
                    info.modes.iter().map(|m| refresh_rate::label(m.hz)).collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                self.modes = info.modes;
                self.selected = info.current as u32;

                // Populate first and connect the handler afterwards so that filling the
                // list does not look like a user selection.
                self.rate_row.set_model(Some(&gtk::StringList::new(&refs)));
                self.rate_row.set_selected(self.selected);
                if !self.handler_connected {
                    self.handler_connected = true;
                    self.rate_row.connect_selected_notify({
                        let sender = sender.clone();
                        move |row| sender.input(RefreshRateMsg::ChangeRate(row.selected()))
                    });
                }

                if self.wanted_hz > 0 {
                    self.apply_wanted(&sender);
                }
            }
            RefreshRateCommandOutput::RateApplied(hz) => {
                tracing::info!("{}", t!("refresh_rate_applied", hz = hz.to_string()));
            }
            RefreshRateCommandOutput::Error(e) => {
                let _ = sender.output(e);
            }
        }
    }
}

impl RefreshRateModel {
    /// Switches to the mode matching `wanted_hz` if the panel offers it and it is not active yet.
    fn apply_wanted(&mut self, sender: &ComponentSender<Self>) {
        let Some(idx) = self
            .modes
            .iter()
            .position(|m| refresh_rate::hz_key(m.hz) == self.wanted_hz)
        else {
            return;
        };
        if idx as u32 == self.selected {
            return;
        }
        self.selected = idx as u32;
        apply_mode(self.modes[idx].clone(), sender);
    }
}

/// Reads the available rates from whichever compositor backend is running.
async fn read_modes() -> Result<Option<RateInfo>, String> {
    if is_gnome_desktop() {
        mutter_display::read_rates(DISPLAY_NAME).await
    } else {
        let json = run_command_stdout("kscreen-doctor", &["-j"]).await?;
        Ok(refresh_rate::parse(&json, DISPLAY_NAME))
    }
}

async fn set_mode(mode_id: &str) -> Result<(), String> {
    if is_gnome_desktop() {
        mutter_display::set_mode(DISPLAY_NAME, mode_id).await
    } else {
        let arg = format!("output.{}.mode.{}", DISPLAY_NAME, mode_id);
        run_command_blocking("kscreen-doctor", &[&arg]).await
    }
}

fn apply_mode(mode: RateMode, sender: &ComponentSender<RefreshRateModel>) {
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                match set_mode(&mode.id).await {
                    Ok(()) => out.emit(RefreshRateCommandOutput::RateApplied(
                        refresh_rate::hz_key(mode.hz),
                    )),
                    Err(e) => out.emit(RefreshRateCommandOutput::Error(e)),
                }
            })
            .drop_on_shutdown()
    });
}
