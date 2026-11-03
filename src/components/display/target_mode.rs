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

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;
use rust_i18n::t;

use super::helpers::run_qdbus;
use crate::services::commands::{is_gnome_desktop, is_kde_desktop, run_command_blocking};
use crate::services::config::AppConfig;
use crate::services::gnome_ext;

pub struct TargetModeModel {
    active: bool,
    kde_available: bool,
    gnome_available: bool,
}

#[derive(Debug)]
pub enum TargetModeMsg {
    SetActive(bool),
    LoadProfile(bool),
}

#[derive(Debug)]
pub enum TargetModeCommandOutput {
    ActiveRead(bool),
    ActiveSet(bool),
    Error(String),
}

#[relm4::component(pub)]
impl Component for TargetModeModel {
    type Init = ();
    type Input = TargetModeMsg;
    type Output = String;
    type CommandOutput = TargetModeCommandOutput;

    view! {
        adw::PreferencesGroup {
            set_title: &t!("target_mode_group_title"),
            set_description: Some(&t!("target_mode_group_desc")),

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                #[watch]
                set_visible: !model.kde_available && !model.gnome_available,
                set_label: &t!("kde_or_gnome_required_warning"),
            },

            #[template]
            add = &crate::components::widgets::DaemonWarningLabel {
                set_visible: model.gnome_available,
                set_label: &t!("gnome_extension_notice"),
            },

            add = &adw::SwitchRow {
                set_title: &t!("target_mode_switch_title"),
                set_subtitle: &t!("target_mode_switch_subtitle"),

                #[watch]
                set_active: model.active,
                #[watch]
                set_sensitive: model.kde_available || model.gnome_available,

                connect_active_notify[sender] => move |switch| {
                    sender.input(TargetModeMsg::SetActive(switch.is_active()));
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
        let kde_available = is_kde_desktop();

        let gnome_available = is_gnome_desktop();

        let model = TargetModeModel {
            active: config.active_profile().target_mode_active,
            kde_available,
            gnome_available,
        };
        let widgets = view_output!();

        if gnome_available && model.active {
            // Re-apply after an update of the bundled extension or a fresh install.
            sender.command(|out, shutdown| {
                shutdown
                    .register(async move {
                        match set_effect(true).await {
                            Ok(()) => out.emit(TargetModeCommandOutput::ActiveSet(true)),
                            Err(e) => out.emit(TargetModeCommandOutput::Error(e)),
                        }
                    })
                    .drop_on_shutdown()
            });
        }

        if kde_available {
            let fallback = config.active_profile().target_mode_active;
            sender.command(move |out, shutdown| {
                shutdown
                    .register(async move {
                        let active = tokio::task::spawn_blocking(move || {
                            read_kwin_bool("Plugins", "diminactiveEnabled")
                        })
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(fallback);
                        out.emit(TargetModeCommandOutput::ActiveRead(active));
                    })
                    .drop_on_shutdown()
            });
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: TargetModeMsg, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            TargetModeMsg::SetActive(active) => {
                if active == self.active {
                    return;
                }
                self.active = active;
                AppConfig::update(|c| c.active_profile_mut().target_mode_active = active);

                sender.command(move |out, shutdown| {
                    shutdown
                        .register(async move {
                            match set_effect(active).await {
                                Ok(()) => out.emit(TargetModeCommandOutput::ActiveSet(active)),
                                Err(e) => out.emit(TargetModeCommandOutput::Error(e)),
                            }
                        })
                        .drop_on_shutdown()
                });
            }
            TargetModeMsg::LoadProfile(active) => {
                if !self.kde_available && !self.gnome_available {
                    return;
                }
                self.active = active;
                sender.command(move |out, shutdown| {
                    shutdown
                        .register(async move {
                            match set_effect(active).await {
                                Ok(()) => out.emit(TargetModeCommandOutput::ActiveSet(active)),
                                Err(e) => out.emit(TargetModeCommandOutput::Error(e)),
                            }
                        })
                        .drop_on_shutdown()
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: TargetModeCommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            TargetModeCommandOutput::ActiveRead(active) => {
                self.active = active;
                AppConfig::update(|c| c.active_profile_mut().target_mode_active = active);
            }
            TargetModeCommandOutput::ActiveSet(active) => {
                tracing::info!(
                    "{}",
                    t!("target_mode_active_set", value = active.to_string())
                );
            }
            TargetModeCommandOutput::Error(e) => {
                let _ = sender.output(e);
            }
        }
    }
}

/// Dims inactive windows: KWin's `diminactive` effect on KDE, the bundled Shell
/// extension on GNOME.
async fn set_effect(active: bool) -> Result<(), String> {
    if is_gnome_desktop() {
        return gnome_ext::set_bool("dim-inactive", active).await;
    }
    let value = if active { "true" } else { "false" };
    run_command_blocking(
        "kwriteconfig6",
        &[
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            "diminactiveEnabled",
            "--type",
            "bool",
            value,
        ],
    )
    .await?;

    let method = if active { "loadEffect" } else { "unloadEffect" };
    run_qdbus(vec![
        "org.kde.KWin".to_string(),
        "/Effects".to_string(),
        method.to_string(),
        "diminactive".to_string(),
    ])
    .await
}

fn read_kwin_bool(group: &str, key: &str) -> Option<bool> {
    let output = std::process::Command::new("kreadconfig6")
        .args([
            "--file",
            "kwinrc",
            "--group",
            group,
            "--key",
            key,
            "--default",
            "false",
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    Some(s == "true")
}
