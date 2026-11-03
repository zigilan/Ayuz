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

use super::cosmic_panel;
use super::helpers::run_qdbus;
use crate::services::commands::{
    is_cosmic_desktop, is_gnome_desktop, is_kde_desktop, run_command_blocking,
};
use crate::services::config::AppConfig;
use crate::services::gnome_ext;
use crate::services::pixel_shift::{self, Orbit};

pub struct OledCareModel {
    pixel_refresh_active: bool,
    panel_autohide_active: bool,
    transparency_active: bool,
    pixel_shift_active: bool,
    pixel_shift_interval: u32,
    /// Stops the running pixel-shift task, which then restores the panel thickness.
    shift_stop: Option<tokio::sync::oneshot::Sender<()>>,
    shift_interval_row: adw::ComboRow,
    kde_available: bool,
    gnome_available: bool,
    panel_ctrl_available: bool,
}

#[derive(Debug)]
pub enum OledCareMsg {
    TogglePixelRefresh(bool),
    TogglePanelAutohide(bool),
    ToggleTransparency(bool),
    TogglePixelShift(bool),
    SetPixelShiftInterval(u32),
    LoadProfile {
        pixel_refresh: bool,
        panel_autohide: bool,
        transparency: bool,
        pixel_shift: bool,
        pixel_shift_interval: u32,
    },
}

#[derive(Debug)]
pub enum OledCareCommandOutput {
    PanelSet(bool),
    TransparencySet(bool),
    PixelRefreshSet(bool),
    PixelShiftFailed(String),
    Error(String),
}

#[relm4::component(pub)]
impl Component for OledCareModel {
    type Init = ();
    type Input = OledCareMsg;
    type Output = String;
    type CommandOutput = OledCareCommandOutput;

    view! {
        adw::PreferencesGroup {
            set_title: &t!("oled_care_group_title"),
            set_description: Some(&t!("oled_care_group_desc")),

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                #[watch]
                set_visible: !model.panel_ctrl_available,
                set_label: &t!("kde_required_warning"),
            },

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                set_visible: model.gnome_available,
                set_label: &t!("gnome_extension_notice"),
            },

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                set_label: &t!("oled_care_group_notice"),
            },

            add = &adw::SwitchRow {
                set_title: &t!("oled_care_pixel_refresh_title"),
                set_subtitle: &t!("oled_care_pixel_refresh_subtitle"),

                #[watch]
                set_active: model.pixel_refresh_active,
                #[watch]
                set_sensitive: model.kde_available || model.gnome_available,

                connect_active_notify[sender] => move |switch| {
                    sender.input(OledCareMsg::TogglePixelRefresh(switch.is_active()));
                },
            },

            add = &adw::SwitchRow {
                set_title: &t!("oled_care_pixel_shift_title"),
                set_subtitle: &t!("oled_care_pixel_shift_subtitle"),

                #[watch]
                set_active: model.pixel_shift_active,
                #[watch]
                set_sensitive: model.kde_available || model.gnome_available,

                connect_active_notify[sender] => move |switch| {
                    sender.input(OledCareMsg::TogglePixelShift(switch.is_active()));
                },
            },

            add = &model.shift_interval_row.clone() -> adw::ComboRow {
                set_title: &t!("oled_care_pixel_shift_interval_title"),

                #[watch]
                set_selected: model.pixel_shift_interval,
                #[watch]
                set_sensitive: (model.kde_available || model.gnome_available)
                    && model.pixel_shift_active,
            },

            add = &adw::SwitchRow {
                set_title: &t!("oled_care_panel_autohide_title"),
                set_subtitle: &t!("oled_care_panel_autohide_subtitle"),

                #[watch]
                set_active: model.panel_autohide_active,
                #[watch]
                set_sensitive: model.panel_ctrl_available,

                connect_active_notify[sender] => move |switch| {
                    sender.input(OledCareMsg::TogglePanelAutohide(switch.is_active()));
                },
            },

            add = &adw::SwitchRow {
                set_title: &t!("oled_care_transparency_title"),
                set_subtitle: &t!("oled_care_transparency_subtitle"),

                #[watch]
                set_active: model.transparency_active,
                #[watch]
                set_sensitive: model.panel_ctrl_available,

                connect_active_notify[sender] => move |switch| {
                    sender.input(OledCareMsg::ToggleTransparency(switch.is_active()));
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = AppConfig::load();
        let p = config.active_profile();
        let kde = is_kde_desktop();
        let cosmic = is_cosmic_desktop();
        let gnome = is_gnome_desktop();

        let interval_labels: Vec<String> = pixel_shift::INTERVALS_SECS
            .iter()
            .map(|s| t!("oled_care_pixel_shift_interval_option", secs = s.to_string()).to_string())
            .collect();
        let interval_refs: Vec<&str> = interval_labels.iter().map(String::as_str).collect();
        let shift_interval_row = adw::ComboRow::new();
        shift_interval_row.set_model(Some(&gtk::StringList::new(&interval_refs)));
        shift_interval_row.connect_selected_notify({
            let sender = sender.clone();
            move |row| sender.input(OledCareMsg::SetPixelShiftInterval(row.selected()))
        });

        let mut model = OledCareModel {
            pixel_refresh_active: p.oled_care_pixel_refresh,
            panel_autohide_active: p.oled_care_panel_autohide,
            transparency_active: p.oled_care_transparency,
            pixel_shift_active: p.oled_care_pixel_shift && (kde || gnome),
            pixel_shift_interval: p.oled_care_pixel_shift_interval,
            shift_stop: None,
            shift_interval_row,
            kde_available: kde,
            gnome_available: gnome,
            panel_ctrl_available: kde || cosmic || gnome,
        };
        let widgets = view_output!();
        if model.pixel_shift_active {
            model.start_pixel_shift(&sender);
        }
        if gnome && (model.panel_autohide_active || model.transparency_active) {
            // Re-apply after an update of the bundled extension or a fresh install.
            gnome_ext_command(&sender, "panel-autohide", model.panel_autohide_active, None);
            gnome_ext_command(&sender, "panel-transparent", model.transparency_active, None);
        }
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: OledCareMsg, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            OledCareMsg::TogglePixelRefresh(active) => {
                if active == self.pixel_refresh_active {
                    return;
                }
                self.pixel_refresh_active = active;

                AppConfig::update(|c| c.active_profile_mut().oled_care_pixel_refresh = active);

                let idle_time = if active { "300" } else { "600" };
                let gnome = self.gnome_available;
                sender.command(move |out, shutdown| {
                    shutdown
                        .register(async move {
                            let result = if gnome {
                                run_command_blocking(
                                    "gsettings",
                                    &["set", "org.gnome.desktop.session", "idle-delay", idle_time],
                                )
                                .await
                            } else {
                                run_command_blocking(
                                    "kwriteconfig6",
                                    &[
                                        "--file",
                                        "powermanagementprofilesrc",
                                        "--group",
                                        "AC",
                                        "--group",
                                        "DPMSControl",
                                        "--key",
                                        "idleTime",
                                        idle_time,
                                    ],
                                )
                                .await
                            };
                            match result {
                                Ok(()) => out.emit(OledCareCommandOutput::PixelRefreshSet(active)),
                                Err(e) => out.emit(OledCareCommandOutput::Error(e)),
                            }
                        })
                        .drop_on_shutdown()
                });
            }
            OledCareMsg::TogglePanelAutohide(active) => {
                if active == self.panel_autohide_active {
                    return;
                }
                self.panel_autohide_active = active;

                AppConfig::update(|c| c.active_profile_mut().oled_care_panel_autohide = active);

                if is_kde_desktop() {
                    let hiding = if active { "autohide" } else { "none" };
                    let script =
                        format!("panels().forEach(function(p){{p.hiding='{}';}})", hiding);
                    sender.command(move |out, shutdown| {
                        shutdown
                            .register(async move {
                                plasmashell_evaluate(
                                    &script,
                                    &out,
                                    OledCareCommandOutput::PanelSet(active),
                                )
                                .await;
                            })
                            .drop_on_shutdown()
                    });
                } else if is_cosmic_desktop() {
                    sender.command(move |out, shutdown| {
                        shutdown
                            .register(async move {
                                match cosmic_panel::set_autohide_all(active).await {
                                    Ok(()) => out.emit(OledCareCommandOutput::PanelSet(active)),
                                    Err(e) => out.emit(OledCareCommandOutput::Error(e)),
                                }
                            })
                            .drop_on_shutdown()
                    });
                } else if self.gnome_available {
                    gnome_ext_command(
                        &sender,
                        "panel-autohide",
                        active,
                        Some(OledCareCommandOutput::PanelSet(active)),
                    );
                }
            }
            OledCareMsg::ToggleTransparency(active) => {
                if active == self.transparency_active {
                    return;
                }
                self.transparency_active = active;

                AppConfig::update(|c| c.active_profile_mut().oled_care_transparency = active);

                if is_kde_desktop() {
                    let opacity = if active { "transparent" } else { "opaque" };
                    let script =
                        format!("panels().forEach(function(p){{p.opacity='{}';}})", opacity);
                    sender.command(move |out, shutdown| {
                        shutdown
                            .register(async move {
                                plasmashell_evaluate(
                                    &script,
                                    &out,
                                    OledCareCommandOutput::TransparencySet(active),
                                )
                                .await;
                            })
                            .drop_on_shutdown()
                    });
                } else if is_cosmic_desktop() {
                    sender.command(move |out, shutdown| {
                        shutdown
                            .register(async move {
                                match cosmic_panel::set_opacity_all(active).await {
                                    Ok(()) => {
                                        out.emit(OledCareCommandOutput::TransparencySet(active))
                                    }
                                    Err(e) => out.emit(OledCareCommandOutput::Error(e)),
                                }
                            })
                            .drop_on_shutdown()
                    });
                } else if self.gnome_available {
                    gnome_ext_command(
                        &sender,
                        "panel-transparent",
                        active,
                        Some(OledCareCommandOutput::TransparencySet(active)),
                    );
                }
            }
            OledCareMsg::TogglePixelShift(active) => {
                if active == self.pixel_shift_active
                    || (active && !self.kde_available && !self.gnome_available)
                {
                    return;
                }
                self.pixel_shift_active = active;

                AppConfig::update(|c| c.active_profile_mut().oled_care_pixel_shift = active);

                if active {
                    self.start_pixel_shift(&sender);
                } else {
                    self.stop_pixel_shift(&sender);
                }
            }
            OledCareMsg::SetPixelShiftInterval(idx) => {
                if idx == self.pixel_shift_interval
                    || idx as usize >= pixel_shift::INTERVALS_SECS.len()
                {
                    return;
                }
                self.pixel_shift_interval = idx;

                AppConfig::update(|c| c.active_profile_mut().oled_care_pixel_shift_interval = idx);

                if self.pixel_shift_active {
                    self.start_pixel_shift(&sender);
                }
            }
            OledCareMsg::LoadProfile {
                pixel_refresh,
                panel_autohide,
                transparency,
                pixel_shift,
                pixel_shift_interval,
            } => {
                if self.kde_available || self.gnome_available {
                    sender.input(OledCareMsg::TogglePixelRefresh(pixel_refresh));
                }
                if self.kde_available || self.gnome_available {
                    sender.input(OledCareMsg::SetPixelShiftInterval(pixel_shift_interval));
                    sender.input(OledCareMsg::TogglePixelShift(pixel_shift));
                }
                if self.panel_ctrl_available {
                    sender.input(OledCareMsg::TogglePanelAutohide(panel_autohide));
                    sender.input(OledCareMsg::ToggleTransparency(transparency));
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: OledCareCommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            OledCareCommandOutput::PanelSet(active) => {
                let value = if active { "autohide" } else { "none" };
                tracing::info!("{}", t!("oled_care_panel_set", value = value));
            }
            OledCareCommandOutput::TransparencySet(active) => {
                let value = if active { "transparent" } else { "opaque" };
                tracing::info!("{}", t!("oled_care_transparency_set", value = value));
            }
            OledCareCommandOutput::PixelRefreshSet(active) => {
                let value = if active { "300s" } else { "600s" };
                tracing::info!("{}", t!("oled_care_dpms_set", value = value));
            }
            OledCareCommandOutput::PixelShiftFailed(e) => {
                self.shift_stop = None;
                self.pixel_shift_active = false;
                AppConfig::update(|c| c.active_profile_mut().oled_care_pixel_shift = false);
                let _ = sender.output(e);
            }
            OledCareCommandOutput::Error(e) => {
                let _ = sender.output(e);
            }
        }
    }
}

impl OledCareModel {
    /// (Re)starts the background task that nudges the KDE panel thickness by one pixel
    /// along the ASUS orbit every configured interval. Moving the panel edge keeps the
    /// brightest static OLED pixels from sitting on the same row indefinitely.
    fn start_pixel_shift(&mut self, sender: &ComponentSender<Self>) {
        if self.gnome_available {
            // The bundled GNOME Shell extension runs the orbit on the top bar itself.
            let secs = pixel_shift::interval_secs(self.pixel_shift_interval) as u32;
            sender.command(move |out, shutdown| {
                shutdown
                    .register(async move {
                        let result = async {
                            gnome_ext::set_uint("pixel-shift-interval", secs).await?;
                            gnome_ext::set_bool("pixel-shift", true).await
                        }
                        .await;
                        if let Err(e) = result {
                            out.emit(OledCareCommandOutput::PixelShiftFailed(e));
                        }
                    })
                    .drop_on_shutdown()
            });
            return;
        }

        self.stop_pixel_shift(sender);

        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        self.shift_stop = Some(stop_tx);
        let interval =
            std::time::Duration::from_secs(pixel_shift::interval_secs(self.pixel_shift_interval));

        sender.command(move |out, shutdown| {
            shutdown
                .register(async move {
                    let mut orbit = Orbit::new();
                    loop {
                        tokio::select! {
                            _ = &mut stop_rx => break,
                            _ = tokio::time::sleep(interval) => {}
                        }
                        let (_, dy) = orbit.advance();
                        if dy != 0 {
                            if let Err(e) = shift_panels(dy).await {
                                out.emit(OledCareCommandOutput::PixelShiftFailed(e));
                                return;
                            }
                        }
                    }
                    // Stopped: put the panels back to their unshifted thickness.
                    let (_, dy) = orbit.offset();
                    if dy != 0 {
                        let _ = shift_panels(-dy).await;
                    }
                })
                .drop_on_shutdown()
        });
    }

    fn stop_pixel_shift(&mut self, sender: &ComponentSender<Self>) {
        if self.gnome_available {
            gnome_ext_command(sender, "pixel-shift", false, None);
            return;
        }
        if let Some(tx) = self.shift_stop.take() {
            let _ = tx.send(());
        }
    }
}

/// Sets a boolean key of the bundled GNOME Shell extension (installing it on first use)
/// and emits `on_success` afterwards, if given.
fn gnome_ext_command(
    sender: &ComponentSender<OledCareModel>,
    key: &'static str,
    value: bool,
    on_success: Option<OledCareCommandOutput>,
) {
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                match gnome_ext::set_bool(key, value).await {
                    Ok(()) => {
                        if let Some(msg) = on_success {
                            out.emit(msg);
                        }
                    }
                    Err(e) => out.emit(OledCareCommandOutput::Error(e)),
                }
            })
            .drop_on_shutdown()
    });
}

/// Changes the thickness of every Plasma panel by `delta` pixels (never below 16 px).
async fn shift_panels(delta: i32) -> Result<(), String> {
    let script = format!(
        "panels().forEach(function(p){{p.height=Math.max(16,p.height+({}));}})",
        delta
    );
    run_qdbus(vec![
        "org.kde.plasmashell".to_string(),
        "/PlasmaShell".to_string(),
        "org.kde.PlasmaShell.evaluateScript".to_string(),
        script,
    ])
    .await
}

async fn plasmashell_evaluate(
    script: &str,
    out: &relm4::Sender<OledCareCommandOutput>,
    success_output: OledCareCommandOutput,
) {
    let args = vec![
        "org.kde.plasmashell".to_string(),
        "/PlasmaShell".to_string(),
        "org.kde.PlasmaShell.evaluateScript".to_string(),
        script.to_string(),
    ];
    match run_qdbus(args).await {
        Ok(()) => out.emit(success_output),
        Err(e) => out.emit(OledCareCommandOutput::Error(e)),
    }
}
