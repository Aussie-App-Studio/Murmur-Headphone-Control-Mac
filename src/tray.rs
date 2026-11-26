#[cfg(target_os = "macos")]
mod macos_tray {
    use std::sync::{mpsc, Arc, Mutex};

    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::controller::{AppCommand, AppController, UiSnapshot};
    use crate::models::{ConnectionStatus, MdrProtocolVersion, NoiseControlMode};

    pub struct MurmurTray {
        tray: TrayIcon,
        item_adaptive: MenuItem,
        item_ambient: MenuItem,
        id_open: MenuId,
        id_anc: MenuId,
        id_adaptive: MenuId,
        id_ambient: MenuId,
        id_off: MenuId,
        id_quit: MenuId,
        menu_rx: mpsc::Receiver<MenuEvent>,
        tray_rx: mpsc::Receiver<TrayIconEvent>,
        egui_ctx: Arc<Mutex<Option<egui::Context>>>,
        last_tooltip: String,
        last_supports_adaptive: bool,
        last_is_apple: bool,
        pub should_quit: bool,
    }

    impl MurmurTray {
        pub fn new() -> Option<Self> {
            let menu = Menu::new();
            let item_open = MenuItem::new("Open Murmur", true, None);
            let item_anc = MenuItem::new("Noise Cancelling", true, None);
            let item_adaptive = MenuItem::new("Adaptive Audio", false, None);
            let item_ambient = MenuItem::new("Ambient Sound", true, None);
            let item_off = MenuItem::new("Noise Control Off", true, None);
            let item_quit = MenuItem::new("Quit Murmur", true, None);

            let id_open = item_open.id().clone();
            let id_anc = item_anc.id().clone();
            let id_adaptive = item_adaptive.id().clone();
            let id_ambient = item_ambient.id().clone();
            let id_off = item_off.id().clone();
            let id_quit = item_quit.id().clone();

            let _ = menu.append(&item_open);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&item_anc);
            let _ = menu.append(&item_adaptive);
            let _ = menu.append(&item_ambient);
            let _ = menu.append(&item_off);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&item_quit);

            let icon = build_template_headphone_icon();
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("Murmur — Headphone Control")
                .with_icon(icon)
                .with_icon_as_template(true)
                .build()
                .ok()?;

            let egui_ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
            let (menu_tx, menu_rx) = mpsc::channel();
            let (tray_tx, tray_rx) = mpsc::channel();

            let ctx_m = Arc::clone(&egui_ctx);
            MenuEvent::set_event_handler(Some(move |ev| {
                let _ = menu_tx.send(ev);
                if let Some(c) = ctx_m.lock().unwrap().as_ref() {
                    c.request_repaint();
                }
            }));

            let ctx_t = Arc::clone(&egui_ctx);
            TrayIconEvent::set_event_handler(Some(move |ev| {
                let _ = tray_tx.send(ev);
                if let Some(c) = ctx_t.lock().unwrap().as_ref() {
                    c.request_repaint();
                }
            }));

            Some(Self {
                tray,
                item_adaptive,
                item_ambient,
                id_open,
                id_anc,
                id_adaptive,
                id_ambient,
                id_off,
                id_quit,
                menu_rx,
                tray_rx,
                egui_ctx,
                last_tooltip: String::new(),
                last_supports_adaptive: false,
                last_is_apple: false,
                should_quit: false,
            })
        }

        pub fn attach_egui_ctx(&self, ctx: egui::Context) {
            *self.egui_ctx.lock().unwrap() = Some(ctx);
        }

        pub fn poll_events(&mut self, controller: &AppController, ctx: &egui::Context) -> bool {
            let mut restored = false;
            while let Ok(event) = self.menu_rx.try_recv() {
                if event.id == self.id_open {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    restored = true;
                } else if event.id == self.id_anc {
                    controller.send(AppCommand::SetNoiseControl {
                        mode: NoiseControlMode::NoiseCancelling,
                        ambient_level: None,
                        focus_on_voice: None,
                        dragging: false,
                    });
                } else if event.id == self.id_adaptive {
                    controller.send(AppCommand::SetNoiseControl {
                        mode: NoiseControlMode::Adaptive,
                        ambient_level: None,
                        focus_on_voice: None,
                        dragging: false,
                    });
                } else if event.id == self.id_ambient {
                    controller.send(AppCommand::SetNoiseControl {
                        mode: NoiseControlMode::AmbientSound,
                        ambient_level: None,
                        focus_on_voice: None,
                        dragging: false,
                    });
                } else if event.id == self.id_off {
                    controller.send(AppCommand::SetNoiseControl {
                        mode: NoiseControlMode::Off,
                        ambient_level: None,
                        focus_on_voice: None,
                        dragging: false,
                    });
                } else if event.id == self.id_quit {
                    self.should_quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    restored = true;
                }
            }

            while let Ok(event) = self.tray_rx.try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
                | TrayIconEvent::DoubleClick { .. } = event
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    restored = true;
                }
            }
            restored
        }

        pub fn update_tooltip(&mut self, snap: &UiSnapshot) {
            let batt = snap
                .headphone
                .battery
                .effective_level()
                .map(|l| format!(" · {}%", l))
                .unwrap_or_default();
            let status = match snap.connection_status {
                ConnectionStatus::Connected => "Connected",
                ConnectionStatus::Connecting => "Connecting",
                ConnectionStatus::Offline => "Offline",
            };
            let tip = format!(
                "Murmur — {} ({}){} · {}",
                snap.headphone.model_name,
                status,
                batt,
                snap.headphone.noise_control.mode.label()
            );
            if tip != self.last_tooltip {
                let _ = self.tray.set_tooltip(Some(&tip));
                self.last_tooltip = tip;
            }

            let supports_adaptive = snap.headphone.capabilities().supports_adaptive_audio;
            if supports_adaptive != self.last_supports_adaptive {
                self.item_adaptive.set_enabled(supports_adaptive);
                self.last_supports_adaptive = supports_adaptive;
            }

            let is_apple = snap.headphone.protocol_version == MdrProtocolVersion::AirPodsAap;
            if is_apple != self.last_is_apple {
                self.item_ambient.set_text(if is_apple {
                    "Transparency"
                } else {
                    "Ambient Sound"
                });
                self.last_is_apple = is_apple;
            }
        }
    }

    fn build_template_headphone_icon() -> Icon {
        let size = 32usize;
        let mut rgba = vec![0u8; size * size * 4];
        let cx = 15.5_f32;
        let cy = 15.5_f32;

        for y in 0..size {
            for x in 0..size {
                let fx = x as f32;
                let fy = y as f32;
                let dx = fx - cx;
                let dy = fy - cy;
                let r = (dx * dx + dy * dy).sqrt();

                let in_arc = fy <= 17.0 && (9.8..=13.2).contains(&r);
                let in_left = (5.0..=10.5).contains(&fx) && (14.5..=25.0).contains(&fy);
                let in_right = (20.5..=26.0).contains(&fx) && (14.5..=25.0).contains(&fy);

                if in_arc || in_left || in_right {
                    let idx = (y * size + x) * 4;
                    // macOS template icons use black pixels with alpha; AppKit adapts for light/dark Menu Bar.
                    rgba[idx] = 0x00;
                    rgba[idx + 1] = 0x00;
                    rgba[idx + 2] = 0x00;
                    rgba[idx + 3] = 0xFF;
                }
            }
        }

        Icon::from_rgba(rgba, size as u32, size as u32).expect("valid icon RGBA")
    }
}

#[cfg(target_os = "macos")]
pub use macos_tray::MurmurTray;

#[cfg(not(target_os = "macos"))]
mod stub {
    use crate::controller::{AppController, UiSnapshot};

    pub struct MurmurTray {
        pub should_quit: bool,
    }

    impl MurmurTray {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn attach_egui_ctx(&self, _ctx: egui::Context) {}
        pub fn poll_events(&mut self, _controller: &AppController, _ctx: &egui::Context) -> bool {
            false
        }
        pub fn update_tooltip(&mut self, _snap: &UiSnapshot) {}
    }
}

#[cfg(not(target_os = "macos"))]
pub use stub::MurmurTray;
