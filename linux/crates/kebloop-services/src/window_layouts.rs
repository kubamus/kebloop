//! Portable saved window layouts. Matching never reuses a live window twice.
use crate::{
    error::ConfigError,
    util::{atomic_write, new_id},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutFrame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWindow {
    pub app_id: String,
    pub title: String,
    pub output: Option<String>,
    pub frame: LayoutFrame,
    pub output_frame: LayoutFrame,
    pub floating: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedWindowLayout {
    pub id: String,
    pub name: String,
    pub windows: Vec<SavedWindow>,
}
#[derive(Debug, Clone)]
pub struct WindowLayouts {
    path: PathBuf,
    layouts: Vec<SavedWindowLayout>,
}
impl WindowLayouts {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref().to_path_buf();
        let layouts = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= 2 * 1024 * 1024 => {
                serde_json::from_slice::<Vec<SavedWindowLayout>>(&bytes)?
            }
            Ok(_) => {
                return Err(ConfigError::invalid(
                    "Saved window layouts file is too large",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(ConfigError::io(&path, error)),
        };
        if layouts.len() > 100 || layouts.iter().any(|layout| !valid(layout)) {
            return Err(ConfigError::invalid("Invalid saved window layout"));
        }
        Ok(Self { path, layouts })
    }
    pub fn layouts(&self) -> &[SavedWindowLayout] {
        &self.layouts
    }
    pub fn save(
        &mut self,
        name: &str,
        windows: Vec<SavedWindow>,
    ) -> Result<SavedWindowLayout, ConfigError> {
        let layout = SavedWindowLayout {
            id: new_id(),
            name: name.trim().into(),
            windows,
        };
        if !valid(&layout) {
            return Err(ConfigError::invalid(
                "Enter a layout name and capture at least one supported window",
            ));
        }
        if self.layouts.len() >= 100 {
            return Err(ConfigError::invalid(
                "At most 100 window layouts can be saved",
            ));
        }
        let mut next = self.layouts.clone();
        next.push(layout.clone());
        self.persist(&next)?;
        self.layouts = next;
        Ok(layout)
    }
    pub fn remove(&mut self, id: &str) -> Result<(), ConfigError> {
        let next: Vec<_> = self
            .layouts
            .iter()
            .filter(|layout| layout.id != id)
            .cloned()
            .collect();
        self.persist(&next)?;
        self.layouts = next;
        Ok(())
    }
    fn persist(&self, layouts: &[SavedWindowLayout]) -> Result<(), ConfigError> {
        atomic_write(&self.path, &serde_json::to_vec_pretty(layouts)?, 0o600)
    }
}
fn valid(layout: &SavedWindowLayout) -> bool {
    !layout.id.is_empty()
        && !layout.name.trim().is_empty()
        && layout.name.len() <= 256
        && !layout.windows.is_empty()
        && layout.windows.len() <= 100
        && layout.windows.iter().all(|window| {
            !window.app_id.is_empty()
                && window.app_id.len() <= 1024
                && window.title.len() <= 4096
                && [window.frame, window.output_frame].iter().all(|frame| {
                    frame.width > 0
                        && frame.height > 0
                        && frame.width <= 100_000
                        && frame.height <= 100_000
                        && frame.x.abs_diff(0) <= 1_000_000
                        && frame.y.abs_diff(0) <= 1_000_000
                })
        })
}
/// Preserve position and size proportionally on a replacement display, then keep it visible.
pub fn adjusted_frame(window: &SavedWindow, destination: LayoutFrame) -> LayoutFrame {
    let destination = LayoutFrame {
        width: destination.width.max(1),
        height: destination.height.max(1),
        ..destination
    };
    let source = window.output_frame;
    let scale = |value: i64, old: i32, new: i32| -> i32 {
        (value * i64::from(new) / i64::from(old.max(1)))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    let width = scale(
        i64::from(window.frame.width),
        source.width,
        destination.width,
    )
    .clamp(1, destination.width.max(1));
    let height = scale(
        i64::from(window.frame.height),
        source.height,
        destination.height,
    )
    .clamp(1, destination.height.max(1));
    let x = destination.x.saturating_add(scale(
        i64::from(window.frame.x) - i64::from(source.x),
        source.width,
        destination.width,
    ));
    let y = destination.y.saturating_add(scale(
        i64::from(window.frame.y) - i64::from(source.y),
        source.height,
        destination.height,
    ));
    LayoutFrame {
        x: x.clamp(
            destination.x,
            destination.x.saturating_add(destination.width - width),
        ),
        y: y.clamp(
            destination.y,
            destination.y.saturating_add(destination.height - height),
        ),
        width,
        height,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn window() -> SavedWindow {
        SavedWindow {
            app_id: "editor".into(),
            title: "doc".into(),
            output: None,
            frame: LayoutFrame {
                x: 500,
                y: 0,
                width: 500,
                height: 800,
            },
            output_frame: LayoutFrame {
                x: 0,
                y: 0,
                width: 1000,
                height: 800,
            },
            floating: true,
        }
    }
    #[test]
    fn saves_reloads_and_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layouts.json");
        let mut store = WindowLayouts::load(&path).unwrap();
        let saved = store.save(" Work ", vec![window()]).unwrap();
        assert_eq!(
            WindowLayouts::load(&path).unwrap().layouts()[0].name,
            "Work"
        );
        store.remove(&saved.id).unwrap();
        assert!(WindowLayouts::load(&path).unwrap().layouts().is_empty());
    }
    #[test]
    fn failed_save_keeps_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("block");
        std::fs::write(&path, "x").unwrap();
        let mut store = WindowLayouts {
            path: path.join("layouts.json"),
            layouts: vec![],
        };
        assert!(store.save("Work", vec![window()]).is_err());
        assert!(store.layouts().is_empty());
    }
    #[test]
    fn adapts_to_removed_display() {
        assert_eq!(
            adjusted_frame(
                &window(),
                LayoutFrame {
                    x: -800,
                    y: 0,
                    width: 800,
                    height: 600
                }
            ),
            LayoutFrame {
                x: -400,
                y: 0,
                width: 400,
                height: 600
            }
        );
    }
}
