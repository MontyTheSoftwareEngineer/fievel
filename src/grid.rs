use crate::{config::HintKeys, input::KeyCode};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridQuadrant {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
}

impl GridQuadrant {
    pub fn key(self, keys: &HintKeys) -> KeyCode {
        match self {
            Self::UpperLeft => keys.grid_upper_left,
            Self::UpperRight => keys.grid_upper_right,
            Self::LowerLeft => keys.grid_lower_left,
            Self::LowerRight => keys.grid_lower_right,
        }
    }

    pub fn from_key(key: KeyCode, keys: &HintKeys) -> Option<Self> {
        [
            Self::UpperLeft,
            Self::UpperRight,
            Self::LowerLeft,
            Self::LowerRight,
        ]
        .into_iter()
        .find(|quadrant| quadrant.key(keys) == key)
    }

    pub fn label(self, keys: &HintKeys) -> String {
        key_label(self.key(keys))
    }
}

pub(crate) fn key_label(key: KeyCode) -> String {
    let punctuation = match key {
        KeyCode::KEY_COMMA => Some(","),
        KeyCode::KEY_DOT => Some("."),
        KeyCode::KEY_SEMICOLON => Some(";"),
        KeyCode::KEY_APOSTROPHE => Some("'"),
        KeyCode::KEY_GRAVE => Some("`"),
        KeyCode::KEY_MINUS => Some("-"),
        KeyCode::KEY_EQUAL => Some("="),
        KeyCode::KEY_LEFTBRACE => Some("["),
        KeyCode::KEY_RIGHTBRACE => Some("]"),
        KeyCode::KEY_BACKSLASH => Some("\\"),
        KeyCode::KEY_SLASH => Some("/"),
        _ => None,
    };
    if let Some(label) = punctuation {
        return label.into();
    }
    let name = format!("{key:?}");
    name.strip_prefix("KEY_")
        .unwrap_or(&name)
        .to_ascii_uppercase()
}

#[derive(Clone, Debug)]
pub struct GridSelection {
    root: GridRect,
    path: Vec<GridQuadrant>,
    cursor: (f64, f64),
}

impl GridSelection {
    pub fn new(root: GridRect) -> Self {
        Self {
            root,
            path: Vec::new(),
            cursor: (root.x + root.width / 2.0, root.y + root.height / 2.0),
        }
    }

    pub fn cursor(&self) -> (f64, f64) {
        self.cursor
    }

    pub fn set_cursor(&mut self, x: f64, y: f64) {
        self.cursor = (x, y);
    }

    pub fn current(&self) -> GridRect {
        self.path.iter().fold(self.root, |rect, quadrant| {
            let half_width = rect.width / 2.0;
            let half_height = rect.height / 2.0;
            match quadrant {
                GridQuadrant::UpperLeft => GridRect {
                    width: half_width,
                    height: half_height,
                    ..rect
                },
                GridQuadrant::UpperRight => GridRect {
                    x: rect.x + half_width,
                    width: half_width,
                    height: half_height,
                    ..rect
                },
                GridQuadrant::LowerLeft => GridRect {
                    y: rect.y + half_height,
                    width: half_width,
                    height: half_height,
                    ..rect
                },
                GridQuadrant::LowerRight => GridRect {
                    x: rect.x + half_width,
                    y: rect.y + half_height,
                    width: half_width,
                    height: half_height,
                    ..rect
                },
            }
        })
    }

    pub fn cells(&self) -> [(GridQuadrant, GridRect); 4] {
        let current = self.current();
        let half_width = current.width / 2.0;
        let half_height = current.height / 2.0;
        [
            (
                GridQuadrant::UpperLeft,
                GridRect {
                    width: half_width,
                    height: half_height,
                    ..current
                },
            ),
            (
                GridQuadrant::UpperRight,
                GridRect {
                    x: current.x + half_width,
                    width: half_width,
                    height: half_height,
                    ..current
                },
            ),
            (
                GridQuadrant::LowerLeft,
                GridRect {
                    y: current.y + half_height,
                    width: half_width,
                    height: half_height,
                    ..current
                },
            ),
            (
                GridQuadrant::LowerRight,
                GridRect {
                    x: current.x + half_width,
                    y: current.y + half_height,
                    width: half_width,
                    height: half_height,
                    ..current
                },
            ),
        ]
    }

    pub fn center(&self) -> (f64, f64) {
        let current = self.current();
        (
            current.x + current.width / 2.0,
            current.y + current.height / 2.0,
        )
    }

    pub fn select(&mut self, quadrant: GridQuadrant) -> bool {
        if self.current().width < 2.0 || self.current().height < 2.0 {
            return false;
        }
        self.path.push(quadrant);
        self.cursor = self.center();
        true
    }

    pub fn backspace(&mut self) -> bool {
        let changed = self.path.pop().is_some();
        if changed {
            self.cursor = self.center();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrants_subdivide_recursively_and_center_tracks_current_cell() {
        let mut selection = GridSelection::new(GridRect {
            x: 10.0,
            y: 20.0,
            width: 800.0,
            height: 600.0,
        });
        assert_eq!(selection.center(), (410.0, 320.0));
        assert_eq!(selection.cursor(), (410.0, 320.0));
        assert!(selection.select(GridQuadrant::UpperRight));
        assert_eq!(
            selection.current(),
            GridRect {
                x: 410.0,
                y: 20.0,
                width: 400.0,
                height: 300.0,
            }
        );
        assert!(selection.select(GridQuadrant::LowerRight));
        assert_eq!(selection.center(), (710.0, 245.0));
        assert_eq!(selection.cursor(), (710.0, 245.0));
        assert!(selection.backspace());
        assert_eq!(selection.center(), (610.0, 170.0));
        assert_eq!(selection.cursor(), (610.0, 170.0));
    }

    #[test]
    fn selection_stops_at_pixel_scale() {
        let mut selection = GridSelection::new(GridRect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        });
        assert!(!selection.select(GridQuadrant::UpperLeft));
    }

    #[test]
    fn configured_keys_resolve_and_label_each_quadrant() {
        let mut keys = HintKeys::default();
        keys.grid_upper_left = KeyCode::KEY_A;
        keys.grid_upper_right = KeyCode::KEY_S;
        keys.grid_lower_left = KeyCode::KEY_D;
        keys.grid_lower_right = KeyCode::KEY_F;
        for (key, quadrant, label) in [
            (KeyCode::KEY_A, GridQuadrant::UpperLeft, "A"),
            (KeyCode::KEY_S, GridQuadrant::UpperRight, "S"),
            (KeyCode::KEY_D, GridQuadrant::LowerLeft, "D"),
            (KeyCode::KEY_F, GridQuadrant::LowerRight, "F"),
        ] {
            assert_eq!(GridQuadrant::from_key(key, &keys), Some(quadrant));
            assert_eq!(quadrant.label(&keys), label);
        }
    }
}
