use super::preferences::Preferences;

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SettingsDraft {
    pub preferences: Preferences,
    pub scale_percent: u16,
    pub autostart: bool,
    pub pack_path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SettingsAction {
    Preview,
    Save,
    Overwrite,
    CheckPack,
    DraftChanged,
    Reset,
    PlaySound,
    RevealPack,
    Exit,
}

// Keep untouched fields in sync and preserve the original value of edited fields.
macro_rules! draft_fields {
    ($visit:ident) => {
        $visit!(preferences.auto_move, "自移动");
        $visit!(preferences.auto_diagonal, "斜向移动");
        $visit!(preferences.fullscreen_hide, "全屏隐藏");
        $visit!(preferences.start_hidden, "启动时隐藏");
        $visit!(preferences.sound, "音效");
        $visit!(preferences.task_bubbles, "任务气泡");
        $visit!(preferences.bubble_scale_percent, "气泡大小");
        $visit!(preferences.passthrough, "鼠标穿透");
        $visit!(preferences.chatgpt_connection, "连接到ChatGPT");
        $visit!(scale_percent, "桌宠大小");
        $visit!(autostart, "登录自启");
        $visit!(pack_path, "素材目录");
    };
}

impl SettingsDraft {
    pub fn rebase(&mut self, base: &mut Self, current: &Self) {
        macro_rules! update {
            ($($field:ident).+, $label:literal) => {
                if self.$($field).+ == base.$($field).+ {
                    self.$($field).+.clone_from(&current.$($field).+);
                    base.$($field).+.clone_from(&current.$($field).+);
                }
            };
        }
        draft_fields!(update);
    }

    pub fn merge(&self, base: &Self, current: &Self, overwrite: bool) -> (Self, Vec<&'static str>) {
        let mut result = current.clone();
        let mut conflicts = Vec::new();
        macro_rules! update {
            ($($field:ident).+, $label:literal) => {
                if self.$($field).+ != base.$($field).+ {
                    if !overwrite && current.$($field).+ != base.$($field).+
                        && current.$($field).+ != self.$($field).+
                    {
                        conflicts.push($label);
                    } else {
                        result.$($field).+.clone_from(&self.$($field).+);
                    }
                }
            };
        }
        draft_fields!(update);
        (result, conflicts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_merges_other_fields_and_reports_overlapping_edits() {
        let base = SettingsDraft {
            preferences: Preferences::default(),
            scale_percent: 100,
            autostart: false,
            pack_path: String::new(),
        };
        let mut draft = base.clone();
        draft.scale_percent = 150;
        let mut current = base.clone();
        current.scale_percent = 110;
        current.preferences.auto_move = false;

        let mut rebased = base.clone();
        draft.rebase(&mut rebased, &current);
        assert!(!draft.preferences.auto_move);
        let (merged, errors) = draft.merge(&rebased, &current, false);
        assert_eq!(merged.scale_percent, 110);
        assert_eq!(errors, ["桌宠大小"]);
        let (merged, errors) = draft.merge(&rebased, &current, true);
        assert_eq!(merged.scale_percent, 150);
        assert!(!merged.preferences.auto_move);
        assert!(errors.is_empty());
    }
}
