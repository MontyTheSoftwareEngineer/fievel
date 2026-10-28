use crate::engine::SpeedMode;
use std::{ffi::CString, io};

unsafe extern "C" {
    fn fievel_ui_init(icon_data: *const u8, icon_len: usize) -> bool;
    fn fievel_ui_set_indicator(visible: bool, locked: bool, label: *const std::ffi::c_char);
}

pub struct Overlays {
    enabled: bool,
    keycast_enabled: bool,
    last_state: Option<(bool, bool, String)>,
}

impl Overlays {
    pub fn new(enabled: bool, keycast_enabled: bool, menu_bar_icon: &[u8]) -> io::Result<Self> {
        if !unsafe { fievel_ui_init(menu_bar_icon.as_ptr(), menu_bar_icon.len()) } {
            return Err(io::Error::other(
                "cannot initialize the macOS menu bar icon; check that FIEVEL_MENU_BAR_ICON points to a valid PNG or ICNS image",
            ));
        }
        Ok(Self {
            enabled,
            keycast_enabled,
            last_state: None,
        })
    }

    pub fn update(
        &mut self,
        active: bool,
        keys: &[evdev::KeyCode],
        speed: SpeedMode,
        locked: bool,
        macro_status: Option<&str>,
    ) {
        let visible = (self.enabled && active) || macro_status.is_some();
        let mut label = if locked {
            "HOLD".to_owned()
        } else {
            "fievel".to_owned()
        };
        if self.keycast_enabled && active && !keys.is_empty() {
            let keycast = keys
                .iter()
                .map(|key| {
                    let name = format!("{key:?}");
                    name.strip_prefix("KEY_")
                        .unwrap_or(&name)
                        .to_ascii_uppercase()
                })
                .collect::<Vec<_>>()
                .join(" ");
            label.push_str("  ");
            label.push_str(&keycast);
        }
        if active {
            match speed {
                SpeedMode::Normal => {}
                SpeedMode::Fast => label.push_str("  FAST"),
                SpeedMode::SuperFast => label.push_str("  SUPER FAST"),
                SpeedMode::Slow => label.push_str("  SLOW"),
            }
        }
        if let Some(status) = macro_status {
            label.push_str("  ");
            label.push_str(status);
        }
        if self
            .last_state
            .as_ref()
            .is_some_and(|(last_visible, last_locked, last_label)| {
                *last_visible == visible && *last_locked == locked && last_label == &label
            })
        {
            return;
        }
        self.last_state = Some((visible, locked, label.clone()));
        let label = CString::new(label).expect("indicator label contains no NUL bytes");
        unsafe { fievel_ui_set_indicator(visible, locked, label.as_ptr()) };
    }

    pub fn set_config(&mut self, enabled: bool, keycast_enabled: bool) {
        self.enabled = enabled;
        self.keycast_enabled = keycast_enabled;
    }

    pub fn clear(&mut self) {
        self.last_state = Some((false, false, String::new()));
        let label = CString::new("").expect("empty label contains no NUL bytes");
        unsafe { fievel_ui_set_indicator(false, false, label.as_ptr()) };
    }
}
