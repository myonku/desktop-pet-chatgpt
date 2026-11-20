use std::{env, io, path::PathBuf, thread, time::Instant};

use winit::event_loop::ActiveEventLoop;

use crate::{
    core::{
        Activity,
        preferences::{INITIAL_SCALE, Preferences, SavedSize},
        settings::{SettingsAction, SettingsDraft},
    },
    media::LoadedResource,
    platform::windows::{Command, reveal_directory, startup_command},
    storage::Config,
};

use super::{App, AppEvent, Scene, assets::open_local_pack, base_dip, probe_dimensions};

pub(super) struct PackCheck {
    draft: SettingsDraft,
    current: bool,
    save: Option<(SettingsDraft, bool)>,
}

pub(crate) struct PackCheckResult(Result<String, String>);

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
        } else if !runtime.preferences.auto_diagonal
            && matches!(self.pet.activity, Activity::Auto(route) if route.vertical_distance != 0)
        {
            self.pet.finish(now);
            self.play_base();
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
    pub(super) fn open_settings(&mut self, notice: Option<String>) {
        if self.settings.as_ref().is_some_and(|panel| !panel.alive()) {
            self.close_settings();
        }
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
        self.open_winui(self.committed_draft(autostart), autostart, notice);
    }

    fn committed_draft(&self, autostart: Option<bool>) -> SettingsDraft {
        let runtime = self.committed.unwrap();
        SettingsDraft {
            preferences: runtime.preferences,
            scale_percent: runtime.size.percent,
            autostart: autostart.unwrap_or(false),
            pack_path: self
                .options
                .startup
                .config
                .pack_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    fn saved_settings(&self) -> SettingsDraft {
        self.committed_draft(self.settings.as_ref().and_then(|panel| panel.autostart))
    }

    pub(super) fn close_settings(&mut self) {
        self.invalidate_pack_check();
        if self.settings.take().is_some() {
            self.restore_runtime();
        }
    }

    fn restore_runtime(&mut self) {
        if let Some(scene) = &mut self.scene
            && let Some(committed) = self.committed
            && let Err(error) = scene.apply_runtime(committed)
        {
            scene.input.settings_error(&error);
        }
    }

    pub(super) fn apply_settings_action(
        &mut self,
        event_loop: &ActiveEventLoop,
        action: SettingsAction,
        draft: &SettingsDraft,
        base: &SettingsDraft,
    ) {
        if self.options.trace_playback {
            eprintln!("Settings action: {action:?}");
        }
        let result = match action {
            SettingsAction::Preview | SettingsAction::DraftChanged => {
                self.cancel_changed_pack_check(draft);
                self.settings.as_mut().unwrap().pending_draft = Some((draft.clone(), base.clone()));
                self.preview_settings(&draft.merge(base, &self.saved_settings(), false).0)
            }
            SettingsAction::Save | SettingsAction::Overwrite => {
                self.save_settings(draft, base, action == SettingsAction::Overwrite)
            }
            SettingsAction::CheckPack => self.check_pack(draft, None),
            SettingsAction::Reset => {
                self.invalidate_pack_check();
                self.reset_settings()
            }
            SettingsAction::RevealPack => reveal_directory(&self.scene.as_ref().unwrap().pack_path)
                .map_err(|error| error.to_string()),
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
            let status = if matches!(
                action,
                SettingsAction::Save | SettingsAction::Overwrite | SettingsAction::Reset
            ) {
                "未保存"
            } else {
                "未完成"
            };
            self.settings_notice(format!("{status}：{error}"));
        }
    }

    fn save_settings(
        &mut self,
        draft: &SettingsDraft,
        base: &SettingsDraft,
        overwrite: bool,
    ) -> Result<(), String> {
        let current = self.saved_settings();
        let (merged, conflicts) = draft.merge(base, &current, overwrite);
        if !conflicts.is_empty() {
            self.settings.as_mut().unwrap().conflict(format!(
                "{}已从快捷面板或托盘更改。可覆盖冲突并保存，或关闭以保留当前设置。",
                conflicts.join("、"),
            ));
            return Ok(());
        }
        if !self.options.startup.development
            && draft_pack_path(&current) != draft_pack_path(&merged)
        {
            self.check_pack(draft, Some((base.clone(), overwrite)))
        } else {
            self.invalidate_pack_check();
            self.save_draft(&merged, draft)
        }
    }

    fn check_pack(
        &mut self,
        draft: &SettingsDraft,
        save: Option<(SettingsDraft, bool)>,
    ) -> Result<(), String> {
        if let Some(check) = &self.pack_check {
            if check.current && &check.draft == draft && check.save == save {
                return Ok(());
            }
            return Err("上一次素材检查仍在进行，请稍后重试。".into());
        }
        let root = match draft_pack_path(draft) {
            Some(path) => self.options.startup.store.root().join(path),
            None => self
                .options
                .startup
                .default_pack()
                .map_err(|error| error.to_string())?,
        };
        let proxy = self.proxy.clone();
        thread::Builder::new()
            .name("settings-pack-check".into())
            .spawn(move || {
                let result = (|| -> io::Result<_> {
                    let pack = open_local_pack(&root)?;
                    pack.validate_playback()?;
                    let manifest = pack.manifest();
                    let name = match pack.load_resource("character.name", None)? {
                        Some(LoadedResource::Text(name)) if !name.trim().is_empty() => {
                            name.trim().chars().take(120).collect()
                        }
                        _ => manifest.pack_id.clone(),
                    };
                    Ok(format!(
                        "{name} · {} · {} 个动作",
                        manifest.pack_version,
                        manifest.clips.len()
                    ))
                })();
                let _ = proxy.send_event(AppEvent::SettingsPackChecked(PackCheckResult(
                    result.map_err(|error| error.to_string()),
                )));
            })
            .map_err(|error| error.to_string())?;
        self.pack_check = Some(PackCheck {
            draft: draft.clone(),
            current: true,
            save,
        });
        self.settings_notice("正在检查素材包…".into());
        Ok(())
    }

    pub(super) fn finish_pack_check(&mut self, PackCheckResult(result): PackCheckResult) {
        let Some(check) = self.pack_check.take().filter(|check| check.current) else {
            return;
        };
        match (result, check.save) {
            (Ok(label), Some((base, overwrite))) => {
                let (merged, conflicts) =
                    check.draft.merge(&base, &self.saved_settings(), overwrite);
                if !conflicts.is_empty() {
                    self.settings.as_mut().unwrap().conflict(format!(
                        "{}在素材检查期间已更改。可覆盖冲突并保存，或关闭以保留当前设置。",
                        conflicts.join("、"),
                    ));
                    return;
                }
                match self.save_draft(&merged, &check.draft) {
                    Ok(()) => self.settings_notice(format!("已保存。重启后载入 {label}。")),
                    Err(error) => self.settings_notice(format!("未保存：{error}")),
                }
            }
            (Ok(label), None) => self.settings_notice(format!("素材包通过检查：{label}。")),
            (Err(error), _) => self.settings_notice(format!("素材包检查失败：{error}")),
        }
    }

    fn invalidate_pack_check(&mut self) {
        if let Some(check) = &mut self.pack_check {
            check.current = false;
        }
    }

    fn cancel_changed_pack_check(&mut self, draft: &SettingsDraft) {
        if let Some(check) = &mut self.pack_check
            && check.current
            && &check.draft != draft
        {
            check.current = false;
            self.settings_notice("草稿已更改，请重新检查或保存。".into());
        }
    }

    fn preview_settings(&mut self, draft: &SettingsDraft) -> Result<(), String> {
        let mut runtime = self.scene.as_ref().unwrap().runtime;
        runtime.preferences = draft.preferences;
        runtime.size.percent = draft.scale_percent;
        self.scene.as_mut().unwrap().apply_runtime(runtime)
    }

    fn save_draft(
        &mut self,
        draft: &SettingsDraft,
        submitted: &SettingsDraft,
    ) -> Result<(), String> {
        let mut runtime = self.scene.as_ref().unwrap().runtime;
        runtime.preferences = draft.preferences;
        runtime.size.percent = draft.scale_percent;
        let mut config = self.options.startup.config.clone();
        if runtime.size != self.committed.unwrap().size {
            config.saved_size = Some(runtime.size);
        }
        let mut pack_changed = false;
        if !self.options.startup.development {
            let path = draft_pack_path(draft);
            if path != config.pack_path {
                config.pack_path = path;
                pack_changed = true;
            }
        }
        config.preferences = runtime.preferences;
        let autostart = self
            .settings
            .as_ref()
            .and_then(|panel| panel.autostart)
            .filter(|saved| *saved != draft.autostart)
            .map(|_| draft.autostart);
        self.commit_settings(config, runtime, autostart, false)?;
        if let (Some(enabled), Some(panel)) = (autostart, &mut self.settings) {
            panel.autostart = Some(enabled);
        }
        let current = self.saved_settings();
        if let Some(panel) = &mut self.settings {
            panel.saved(submitted.clone(), current);
        }
        self.refresh_quick_values();
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
        let draft = SettingsDraft {
            preferences: runtime.preferences,
            scale_percent: INITIAL_SCALE,
            autostart: false,
            pack_path: String::new(),
        };
        if let Some(panel) = &mut self.settings {
            panel.reset(draft);
        }
        self.refresh_quick_values();
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
            self.clients
                .set_chatgpt_enabled(runtime.preferences.chatgpt_connection);
            self.refresh_client_panels(Instant::now());
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
                self.clients
                    .set_chatgpt_enabled(runtime.preferences.chatgpt_connection);
                self.refresh_client_panels(Instant::now());
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
            Some(Command::OpenSettings) => self.open_settings(None),
            Some(Command::OpenHub) => self.open_hub(),
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
                self.commit_quick(config, runtime);
            }
            _ => {}
        }
    }

    pub(super) fn save_quick_scale(&mut self, percent: u16) {
        let mut runtime = self.committed.unwrap();
        if runtime.size.percent == percent {
            return;
        }
        runtime.size.percent = percent;
        let mut config = self.options.startup.config.clone();
        config.saved_size = Some(runtime.size);
        self.commit_quick(config, runtime);
    }

    fn commit_quick(&mut self, config: Config, runtime: RuntimeSettings) {
        if let Err(error) = self.commit_settings(config, runtime, None, false) {
            self.open_settings(Some(format!("未保存：{error}")));
            return;
        }
        self.refresh_quick_values();
        let current = self.saved_settings();
        if let Some(panel) = &mut self.settings {
            panel.notify_committed(current.clone());
            if let Some((draft, base)) = &panel.pending_draft {
                let merged = draft.merge(base, &current, false).0;
                if let Err(error) = self.preview_settings(&merged) {
                    self.settings_notice(error);
                }
            }
        }
    }

    pub(super) fn settings_notice(&mut self, message: String) {
        if let Some(panel) = &mut self.settings {
            panel.notice(message);
        }
    }
}

fn draft_pack_path(draft: &SettingsDraft) -> Option<PathBuf> {
    (!draft.pack_path.is_empty()).then(|| PathBuf::from(&draft.pack_path))
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
