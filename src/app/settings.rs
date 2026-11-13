use std::{env, io, path::PathBuf, time::Instant};

use winit::{event::WindowEvent, event_loop::ActiveEventLoop};

use crate::{
    core::{
        Activity,
        preferences::{INITIAL_SCALE, Preferences, SavedSize},
    },
    platform::windows::{Command, local_path, reveal_directory, startup_command},
    storage::Config,
    ui::{SettingsAction, SettingsDraft, SettingsWindow},
};

use super::{App, Scene, base_dip, probe_dimensions};

#[derive(Clone, Copy)]
pub(super) struct RuntimeSettings {
    pub preferences: Preferences,
    pub size: SavedSize,
}

impl Scene {
    fn apply_runtime(&mut self, runtime: RuntimeSettings) -> Result<(), String> {
        let passthrough = runtime.preferences.passthrough && self.input.has_tray();
        self.set_passthrough(passthrough)?;
        let now = Instant::now();
        if runtime.preferences.auto_move != self.pet.auto_enabled {
            self.pet.auto_enabled = runtime.preferences.auto_move;
            if matches!(self.pet.activity, Activity::Base | Activity::Auto(_)) {
                self.pet.reset(now);
                self.play_base();
            }
        }
        if !runtime.preferences.sound {
            self.sound.stop();
        }
        self.runtime = runtime;
        self.update_display(now);
        Ok(())
    }

    pub(super) fn play_click_sound(&mut self) {
        if self.runtime.preferences.sound
            && self.visible
            && !self.minimized
            && self.sound.play()
            && self.trace.is_some()
        {
            eprintln!("Sound: interaction.click");
        }
    }

    fn default_runtime(&self) -> RuntimeSettings {
        let display = self.input.display();
        let mm = self.probe_edid.then(|| probe_dimensions(display)).flatten();
        RuntimeSettings {
            preferences: Preferences::default(),
            size: SavedSize {
                base_dip: base_dip(display, mm, self.window.scale_factor()),
                percent: INITIAL_SCALE,
            },
        }
    }
}

impl App {
    pub(super) fn open_settings(&mut self, event_loop: &ActiveEventLoop, notice: Option<String>) {
        if let Some(settings) = &mut self.settings {
            if notice.is_some() {
                settings.notice = notice;
            }
            settings.window().set_minimized(false);
            settings.window().focus_window();
            settings.window().request_redraw();
            return;
        }
        let scene = self.scene.as_ref().unwrap();
        let startup = &self.options.startup;
        let autostart = self
            .autostart
            .as_ref()
            .filter(|_| !startup.development)
            .and_then(|entry| match entry.read() {
                Ok(value) => Some(value.is_some()),
                Err(error) => {
                    eprintln!("Could not read Windows autostart: {error}");
                    None
                }
            });
        let draft = SettingsDraft {
            preferences: scene.runtime.preferences,
            scale_percent: scene.runtime.size.percent,
            autostart: autostart.unwrap_or(false),
            pack_path: startup
                .config
                .pack_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        };
        match SettingsWindow::open(
            event_loop,
            draft,
            startup.development,
            scene.trace.is_some(),
        ) {
            Ok(mut settings) => {
                settings.notice = notice;
                settings.saved_autostart = autostart;
                self.settings = Some(settings);
                self.refresh_settings_character();
            }
            Err(error) => {
                eprintln!("Could not open settings: {error}");
                scene.input.settings_error("无法打开设置窗口。");
            }
        }
    }

    pub(super) fn refresh_settings_character(&mut self) {
        let (Some(scene), Some(settings)) = (&self.scene, &mut self.settings) else {
            return;
        };
        let fallback;
        let poster = if let Some(poster) = &scene.assets.poster {
            poster
        } else {
            fallback = egui::ColorImage::from_rgba_unmultiplied(
                [scene.sample.width as usize, scene.sample.height as usize],
                &scene.sample.rgba,
            );
            &fallback
        };
        settings.character(scene.assets.name.as_deref(), poster);
    }

    pub(super) fn close_settings(&mut self) {
        if self.settings.take().is_some()
            && let Some(scene) = &mut self.scene
            && let Some(committed) = self.committed
            && let Err(error) = scene.apply_runtime(committed)
        {
            scene.input.settings_error(&error);
        }
    }

    pub(super) fn settings_event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            self.close_settings();
            return;
        }
        let scene = self.scene.as_ref().unwrap();
        let settings = self.settings.as_mut().unwrap();
        let action = settings.event(
            event,
            scene.input.has_tray(),
            scene.sound.available(),
            &scene.pack_status,
        );
        let Some(action) = action else {
            return;
        };
        if scene.trace.is_some() {
            eprintln!("Settings action: {action:?}");
        }
        let draft = settings.draft.clone();
        let result = match action {
            SettingsAction::Preview => self.preview_settings(&draft),
            SettingsAction::Save => self.save_draft(&draft),
            SettingsAction::Reset => self.reset_settings(),
            SettingsAction::RevealPack => {
                reveal_directory(&scene.pack_path).map_err(|error| error.to_string())
            }
            SettingsAction::PlaySound => {
                let scene = self.scene.as_mut().unwrap();
                if scene.sound.play() {
                    if scene.trace.is_some() {
                        eprintln!("Sound: interaction.click (settings)");
                    }
                    Ok(())
                } else {
                    Err("无法播放当前素材包的音效。".into())
                }
            }
            SettingsAction::Exit => {
                event_loop.exit();
                return;
            }
        };
        if let Err(error) = result {
            if self.options.trace_playback {
                eprintln!("Settings error: {error}");
            }
            let status = if matches!(action, SettingsAction::Save | SettingsAction::Reset) {
                "未保存"
            } else {
                "未完成"
            };
            self.settings_notice(format!("{status}：{error}"));
        }
    }

    fn preview_settings(&mut self, draft: &SettingsDraft) -> Result<(), String> {
        let mut runtime = self.scene.as_ref().unwrap().runtime;
        runtime.preferences = draft.preferences;
        runtime.size.percent = draft.scale_percent;
        self.scene.as_mut().unwrap().apply_runtime(runtime)
    }

    fn save_draft(&mut self, draft: &SettingsDraft) -> Result<(), String> {
        self.preview_settings(draft)?;
        let runtime = self.scene.as_ref().unwrap().runtime;
        let mut config = self.options.startup.config.clone();
        if runtime.size != self.committed.unwrap().size {
            config.saved_size = Some(runtime.size);
        }
        let mut pack_changed = false;
        if !self.options.startup.development {
            let path = (!draft.pack_path.is_empty()).then(|| PathBuf::from(&draft.pack_path));
            if path != config.pack_path {
                if let Some(path) = &path {
                    let resolved = self.options.startup.store.root().join(path);
                    local_path(&resolved).map_err(|error| error.to_string())?;
                }
                config.pack_path = path;
                pack_changed = true;
            }
        }
        config.preferences = runtime.preferences;
        let autostart = self
            .settings
            .as_ref()
            .and_then(|window| window.saved_autostart)
            .filter(|saved| *saved != draft.autostart)
            .map(|_| draft.autostart);
        self.commit_settings(config, runtime, autostart, false)?;
        if let Some(enabled) = autostart {
            self.settings.as_mut().unwrap().saved_autostart = Some(enabled);
        }
        self.settings_notice(if self.options.startup.development {
            "已应用到本次运行。".into()
        } else if pack_changed {
            "已保存。重启桌宠后载入新的素材目录。".into()
        } else {
            "已保存。".into()
        });
        Ok(())
    }

    fn reset_settings(&mut self) -> Result<(), String> {
        let runtime = self.scene.as_ref().unwrap().default_runtime();
        let backup = self.commit_settings(Config::default(), runtime, Some(false), true)?;
        if let Some(settings) = &mut self.settings {
            settings.draft = SettingsDraft {
                preferences: runtime.preferences,
                scale_percent: INITIAL_SCALE,
                autostart: false,
                pack_path: String::new(),
            };
            settings.saved_autostart = settings.saved_autostart.map(|_| false);
        }
        let notice = if self.options.startup.development {
            "已恢复本次运行的默认设置。".into()
        } else if let Some(path) = backup {
            format!(
                "已恢复默认值，原配置已备份到 {}。素材包在重启后生效。",
                path.display()
            )
        } else {
            "已恢复默认值。素材包在重启后生效。".into()
        };
        self.settings_notice(notice);
        Ok(())
    }

    fn commit_settings(
        &mut self,
        config: Config,
        runtime: RuntimeSettings,
        autostart: Option<bool>,
        repair: bool,
    ) -> Result<Option<PathBuf>, String> {
        let scene = self.scene.as_mut().unwrap();
        let before = scene.runtime;
        scene.apply_runtime(runtime)?;
        if self.options.startup.development {
            self.committed = Some(runtime);
            return Ok(None);
        }
        let result = (|| -> Result<_, Box<dyn std::error::Error>> {
            // The Run value is the source of truth. Roll it back if TOML replacement fails.
            let mut old_value = None;
            if let (Some(enabled), Some(entry)) = (autostart, &self.autostart) {
                let previous = entry.read()?;
                if enabled != previous.is_some() {
                    let new_value = enabled
                        .then(|| env::current_exe().and_then(|path| startup_command(&path)))
                        .transpose()?;
                    entry.write(new_value.as_deref())?;
                    old_value = Some(previous);
                }
            }
            match self.options.startup.store.save(&config, repair) {
                Ok(backup) => Ok(backup),
                Err(error) => {
                    if let (Some(previous), Some(entry)) = (old_value, &self.autostart)
                        && let Err(rollback) = entry.write(previous.as_deref())
                    {
                        return Err(io::Error::other(format!(
                            "{error}；登录自启回滚失败：{rollback}"
                        ))
                        .into());
                    }
                    Err(error.into())
                }
            }
        })();
        match result {
            Ok(backup) => {
                self.options.startup.config = config;
                self.committed = Some(runtime);
                Ok(backup)
            }
            Err(error) => {
                if let Err(rollback) = scene.apply_runtime(before) {
                    return Err(format!("{error}；运行时设置回滚失败：{rollback}"));
                }
                Err(error.to_string())
            }
        }
    }

    pub(super) fn handle_menu(&mut self, event_loop: &ActiveEventLoop, command: Option<Command>) {
        match command {
            Some(Command::OpenSettings) => self.open_settings(event_loop, None),
            #[cfg(feature = "window-diagnostics")]
            Some(Command::OpenPreview) => self.open_preview(event_loop),
            Some(Command::Exit) => event_loop.exit(),
            Some(
                command @ (Command::ToggleAuto
                | Command::ToggleSound
                | Command::ToggleFullscreen
                | Command::TogglePassthrough),
            ) => {
                let mut runtime = self.committed.unwrap();
                toggle(&mut runtime.preferences, command);
                let mut config = self.options.startup.config.clone();
                config.preferences = runtime.preferences;
                if let Err(error) = self.commit_settings(config, runtime, None, false) {
                    self.open_settings(event_loop, Some(format!("未保存：{error}")));
                }
            }
            _ => {}
        }
    }

    fn settings_notice(&mut self, message: String) {
        if let Some(settings) = &mut self.settings {
            settings.notice = Some(message);
            settings.window().request_redraw();
        }
    }
}

fn toggle(preferences: &mut Preferences, command: Command) {
    let flag = match command {
        Command::ToggleAuto => &mut preferences.auto_move,
        Command::ToggleFullscreen => &mut preferences.fullscreen_hide,
        Command::ToggleSound => &mut preferences.sound,
        Command::TogglePassthrough => &mut preferences.passthrough,
        _ => return,
    };
    *flag = !*flag;
}
