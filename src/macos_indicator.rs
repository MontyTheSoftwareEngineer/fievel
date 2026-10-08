use crate::engine::SpeedMode;
use std::ffi::CString;

unsafe extern "C" {
    fn fievel_ui_init();
    fn fievel_ui_set_indicator(visible: bool, locked: bool, label: *const std::ffi::c_char);
}

pub struct Overlays {
    enabled: bool,
    keycast_enabled: bool,
}

impl Overlays {
    pub fn new(enabled: bool, keycast_enabled: bool) -> Self {
        unsafe { fievel_ui_init() };
        Self {
            enabled,
            keycast_enabled,
        }
    }

    pub fn update(
        &mut self,
        active: bool,
        keys: &[evdev::KeyCode],
        speed: SpeedMode,
        locked: bool,
    ) {
        let visible = self.enabled && active;
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
                SpeedMode::Slow => label.push_str("  SLOW"),
            }
        }
        let label = CString::new(label).expect("indicator label contains no NUL bytes");
        unsafe { fievel_ui_set_indicator(visible, locked, label.as_ptr()) };
    }

    pub fn clear(&mut self) {
        let label = CString::new("").expect("empty label contains no NUL bytes");
        unsafe { fievel_ui_set_indicator(false, false, label.as_ptr()) };
    }
}
