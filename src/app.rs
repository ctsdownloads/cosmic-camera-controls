use std::collections::HashMap;
use std::time::Duration;

use cosmic::app::{Core, Task};
use cosmic::iced::{Length, Subscription};
use cosmic::widget::{self, column, container, dropdown, row, slider, text, toggler};
use cosmic::{Application, Element};

use crate::camera::{self, CameraControl, CameraInfo, ControlKind, FormatOption};
use crate::config::{CameraProfile, Config, SavedFormat};
use crate::preview::PreviewHandle;

pub const APP_ID: &str = "com.github.cosmic-camera-controls";

pub struct App {
    core: Core,
    cameras: Vec<CameraInfo>,
    selected_camera: Option<usize>,
    dev_path: Option<std::path::PathBuf>,
    controls: Vec<CameraControl>,
    control_values: HashMap<u32, i64>,
    formats: Vec<FormatOption>,
    selected_format: Option<usize>,
    config: Config,
    status: String,
    camera_labels: Vec<String>,
    format_labels: Vec<String>,
    // Preview
    preview: Option<PreviewHandle>,
    preview_frame: Option<cosmic::widget::image::Handle>,
    preview_width: u32,
    preview_height: u32,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectCamera(usize),
    ControlChanged(u32, i64),
    ControlToggled(u32, bool),
    SelectFormat(usize),
    ResetDefaults,
    PollPreview,
    CheckDevices,
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let cameras = camera::enumerate_cameras();
        let config = Config::load();

        let mut app = App {
            core,
            cameras,
            selected_camera: None,
            dev_path: None,
            controls: Vec::new(),
            control_values: HashMap::new(),
            formats: Vec::new(),
            selected_format: None,
            config,
            status: String::new(),
            camera_labels: Vec::new(),
            format_labels: Vec::new(),
            preview: None,
            preview_frame: None,
            preview_width: 0,
            preview_height: 0,
        };

        app.rebuild_camera_labels();

        if !app.cameras.is_empty() {
            app.open_camera(0);
        } else {
            app.status = "No cameras detected".into();
        }

        (app, Task::none())
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::SelectCamera(idx) => {
                self.open_camera(idx);
            }
            Message::ControlChanged(id, value) => {
                if let Some(path) = &self.dev_path {
                    match camera::set_control_value(path, id, value) {
                        Ok(()) => {
                            self.control_values.insert(id, value);
                            self.auto_save();
                        }
                        Err(e) if e.is_permission_denied() => {
                            // Auto-managed control — revert slider to actual value
                            if let Some(actual) = camera::get_control_value(path, id) {
                                self.control_values.insert(id, actual);
                            }
                        }
                        Err(e) => {
                            self.status = e.message;
                        }
                    }
                }
            }
            Message::ControlToggled(id, on) => {
                let value = if on { 1 } else { 0 };
                if let Some(path) = &self.dev_path {
                    match camera::set_control_value(path, id, value) {
                        Ok(()) => {
                            self.control_values.insert(id, value);
                            self.auto_save();
                        }
                        Err(e) if e.is_permission_denied() => {}
                        Err(e) => {
                            self.status = e.message;
                        }
                    }
                }
            }
            Message::SelectFormat(idx) => {
                self.selected_format = Some(idx);
                if let (Some(path), Some(fmt)) =
                    (&self.dev_path, self.formats.get(idx))
                {
                    let path = path.clone();
                    let fourcc = fmt.fourcc;
                    let width = fmt.width;
                    let height = fmt.height;

                    // Stop preview before changing format
                    self.preview = None;
                    self.preview_frame = None;

                    match camera::set_format(&path, fourcc, width, height) {
                        Ok((actual_w, actual_h)) => {
                            self.status = format!(
                                "Format: {} {}x{}",
                                fourcc, actual_w, actual_h
                            );

                            // Restart preview with new format
                            match PreviewHandle::start(path) {
                                Ok(handle) => {
                                    self.preview = Some(handle);
                                }
                                Err(e) => {
                                    self.status = format!("Preview failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            self.status = e;
                            // Try to restart preview with old format
                            if let Ok(handle) = PreviewHandle::start(path) {
                                self.preview = Some(handle);
                            }
                        }
                    }
                }
                self.auto_save();
            }
            Message::ResetDefaults => {
                self.reset_to_defaults();
            }
            Message::PollPreview => {
                if let Some(ref preview) = self.preview {
                    // Drain to latest frame
                    let mut latest = None;
                    while let Ok(frame) = preview.rx.try_recv() {
                        latest = Some(frame);
                    }
                    if let Some(frame) = latest {
                        self.preview_width = frame.width;
                        self.preview_height = frame.height;
                        self.preview_frame = Some(
                            cosmic::widget::image::Handle::from_rgba(
                                frame.width,
                                frame.height,
                                frame.rgba,
                            ),
                        );
                    }
                }
            }
            Message::CheckDevices => {
                let current: Vec<std::path::PathBuf> = camera::enumerate_cameras()
                    .iter()
                    .map(|c| c.dev_path.clone())
                    .collect();
                let known: Vec<std::path::PathBuf> = self
                    .cameras
                    .iter()
                    .map(|c| c.dev_path.clone())
                    .collect();

                if current != known {
                    log::info!("Device change detected: {:?} -> {:?}", known, current);

                    // Stop preview before re-enumerating
                    self.preview = None;
                    self.preview_frame = None;

                    self.cameras = camera::enumerate_cameras();
                    self.rebuild_camera_labels();

                    // If current camera disappeared, switch to first available
                    let current_path = self.dev_path.clone();
                    let still_exists = current_path
                        .as_ref()
                        .map(|p| self.cameras.iter().any(|c| &c.dev_path == p))
                        .unwrap_or(false);

                    if !still_exists {
                        self.selected_camera = None;
                        self.dev_path = None;
                        self.controls.clear();
                        self.control_values.clear();
                        self.formats.clear();
                        self.format_labels.clear();

                        if !self.cameras.is_empty() {
                            self.open_camera(0);
                        } else {
                            self.status = "No cameras detected".into();
                        }
                    } else {
                        // Camera list changed but ours is still there — just rebuild labels
                        self.selected_camera = self
                            .cameras
                            .iter()
                            .position(|c| Some(&c.dev_path) == current_path.as_ref());
                    }
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let device_check = cosmic::iced::time::every(Duration::from_secs(2))
            .map(|_| Message::CheckDevices);

        if self.preview.is_some() {
            let preview_poll = cosmic::iced::time::every(Duration::from_millis(66))
                .map(|_| Message::PollPreview);
            Subscription::batch([preview_poll, device_check])
        } else {
            device_check
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let mut sections: Vec<Element<Self::Message>> = Vec::new();

        // Camera selector
        if self.cameras.len() > 1 {
            sections.push(
                row([
                    text::title4("Device").into(),
                    dropdown(
                        &self.camera_labels,
                        self.selected_camera,
                        Message::SelectCamera,
                    )
                    .into(),
                ])
                .spacing(8)
                .into(),
            );
        } else if let Some(cam) = self.cameras.first() {
            sections.push(
                text::title4(format!("{} — {}", cam.name, cam.dev_path.display())).into(),
            );
        }

        // Live preview
        if let Some(handle) = &self.preview_frame {
            sections.push(
                container(
                    cosmic::widget::image(handle.clone())
                        .width(Length::Fixed(480.0))
                        .height(Length::Fixed(
                            if self.preview_width > 0 {
                                480.0 * self.preview_height as f32 / self.preview_width as f32
                            } else {
                                360.0
                            },
                        )),
                )
                .width(Length::Fill)
                .center_x(Length::Fill)
                .into(),
            );
        } else if self.selected_camera.is_some() {
            sections.push(text("Starting preview...").into());
        }

        // Format selector
        if !self.format_labels.is_empty() {
            sections.push(
                row([
                    text::title4("Format").into(),
                    dropdown(
                        &self.format_labels,
                        self.selected_format,
                        Message::SelectFormat,
                    )
                    .into(),
                ])
                .spacing(8)
                .into(),
            );
        }

        // Dynamic controls
        if !self.controls.is_empty() {
            sections.push(text::title4("Controls").into());

            for ctrl in &self.controls {
                let current = self
                    .control_values
                    .get(&ctrl.id)
                    .copied()
                    .unwrap_or(ctrl.default);

                let control_row: Element<Self::Message> = match &ctrl.ctrl_type {
                    ControlKind::Integer { min, max, step } => {
                        let id = ctrl.id;
                        let step_val = *step;
                        row([
                            text(&ctrl.name).width(Length::Fixed(200.0)).into(),
                            slider(*min as f64..=*max as f64, current as f64, move |v| {
                                let stepped = if step_val > 1 {
                                    let s = step_val as f64;
                                    (v / s).round() as i64 * step_val
                                } else {
                                    v as i64
                                };
                                Message::ControlChanged(id, stepped)
                            })
                            .width(Length::Fill)
                            .into(),
                            text(format!("{}", current))
                                .width(Length::Fixed(60.0))
                                .into(),
                        ])
                        .spacing(12)
                        .align_y(cosmic::iced::Alignment::Center)
                        .into()
                    }
                    ControlKind::Boolean => {
                        let id = ctrl.id;
                        row([
                            text(&ctrl.name).width(Length::Fixed(200.0)).into(),
                            toggler(current != 0)
                                .on_toggle(move |on| Message::ControlToggled(id, on))
                                .into(),
                        ])
                        .spacing(12)
                        .align_y(cosmic::iced::Alignment::Center)
                        .into()
                    }
                    ControlKind::Menu { items } => {
                        let current_label = items
                            .iter()
                            .find(|(val, _)| *val == current)
                            .map(|(_, label)| label.as_str())
                            .unwrap_or("Unknown");

                        row([
                            text(&ctrl.name).width(Length::Fixed(200.0)).into(),
                            text(current_label).into(),
                        ])
                        .spacing(12)
                        .align_y(cosmic::iced::Alignment::Center)
                        .into()
                    }
                    ControlKind::Button => {
                        let id = ctrl.id;
                        row([widget::button::standard(&ctrl.name)
                            .on_press(Message::ControlChanged(id, 1))
                            .into()])
                        .spacing(12)
                        .into()
                    }
                };

                sections.push(control_row);
            }
        }

        // Action buttons
        sections.push(
            widget::button::standard("Reset Defaults")
                .on_press(Message::ResetDefaults)
                .into(),
        );

        // Status
        if !self.status.is_empty() {
            sections.push(text(&self.status).into());
        }

        container(
            widget::scrollable(
                column(sections).spacing(16).padding(24).width(Length::Fill),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

impl App {
    fn rebuild_camera_labels(&mut self) {
        self.camera_labels = self
            .cameras
            .iter()
            .map(|c| format!("{} ({})", c.name, c.dev_path.display()))
            .collect();
    }

    fn rebuild_format_labels(&mut self) {
        self.format_labels = self
            .formats
            .iter()
            .map(|f| {
                let fps_str = f
                    .framerates
                    .first()
                    .map(|(n, d)| {
                        if *n > 0 {
                            format!(" @ {}fps", d / n)
                        } else {
                            String::new()
                        }
                    })
                    .unwrap_or_default();
                format!("{} {}x{}{}", f.fourcc, f.width, f.height, fps_str)
            })
            .collect();
    }

    fn open_camera(&mut self, idx: usize) {
        let Some(cam) = self.cameras.get(idx) else {
            return;
        };

        // Clone what we need before mutably borrowing self
        let dev_path = cam.dev_path.clone();
        let cam_name = cam.name.clone();
        let profile_key = cam.id.profile_key();

        // Stop existing preview
        self.preview = None;
        self.preview_frame = None;

        match v4l::device::Device::with_path(&dev_path) {
            Ok(dev) => {
                self.controls = camera::enumerate_controls(&dev_path);
                self.formats = camera::enumerate_formats(&dev);
                self.control_values = camera::snapshot_controls(&dev_path, &self.controls);
                self.selected_camera = Some(idx);
                self.rebuild_format_labels();

                if let Some(profile) = self.config.get_profile(&profile_key) {
                    let restored: HashMap<u32, i64> = profile
                        .controls
                        .iter()
                        .filter_map(|(k, v)| k.parse::<u32>().ok().map(|id| (id, *v)))
                        .collect();
                    let errors = camera::apply_controls(&dev_path, &restored, &self.controls);
                    // Always re-read actual values after apply
                    self.control_values = camera::snapshot_controls(&dev_path, &self.controls);
                    if errors.is_empty() {
                        self.status = format!("Profile restored for {}", cam_name);
                    } else {
                        let detail = errors.join("; ");
                        log::warn!("Profile apply errors: {}", detail);
                        self.status = format!("Profile restored — failed: {}", detail);
                    }
                } else {
                    self.status = format!("Opened {}", cam_name);
                }

                // Drop the v4l Device so preview thread can open it
                drop(dev);
                self.dev_path = Some(dev_path.clone());

                // Start preview
                match PreviewHandle::start(dev_path) {
                    Ok(handle) => {
                        self.preview = Some(handle);
                    }
                    Err(e) => {
                        log::error!("Failed to start preview: {}", e);
                        self.status = format!("Preview failed: {}", e);
                    }
                }
            }
            Err(e) => {
                self.status = format!("Failed to open {}: {}", dev_path.display(), e);
            }
        }
    }

    fn auto_save(&mut self) {
        self.save_current_profile();
    }

    fn save_current_profile(&mut self) {
        let Some(idx) = self.selected_camera else {
            return;
        };
        let Some(cam) = self.cameras.get(idx) else {
            return;
        };

        let saved_format = self.selected_format.and_then(|fi| {
            self.formats.get(fi).map(|f| SavedFormat {
                fourcc: f.fourcc.to_string(),
                width: f.width,
                height: f.height,
                framerate_num: f.framerates.first().map(|r| r.0).unwrap_or(1),
                framerate_den: f.framerates.first().map(|r| r.1).unwrap_or(30),
            })
        });

        let profile = CameraProfile {
            name: cam.name.clone(),
            controls: self
                .control_values
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect(),
            format: saved_format,
        };

        let key = cam.id.profile_key();
        self.config.set_profile(key, profile);

        match self.config.save() {
            Ok(()) => {}
            Err(e) => self.status = format!("Save failed: {}", e),
        }
    }

    fn reset_to_defaults(&mut self) {
        let Some(path) = &self.dev_path else {
            return;
        };
        let path = path.clone();

        let mut defaults: HashMap<u32, i64> = HashMap::new();
        for ctrl in &self.controls {
            defaults.insert(ctrl.id, ctrl.default);
        }

        let errors = camera::apply_controls(&path, &defaults, &self.controls);
        self.control_values = camera::snapshot_controls(&path, &self.controls);
        self.auto_save();

        if errors.is_empty() {
            self.status = "Reset to defaults".into();
        } else {
            let detail = errors.join("; ");
            log::warn!("Reset errors: {}", detail);
            self.status = format!("Reset — failed: {}", detail);
        }
    }
}
