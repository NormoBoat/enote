use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    NewFile,
    NewFolder,
    Open,
    Save,
    SaveAs,
    CloseTab,
    Undo,
    Redo,
    NextTab,
    PreviousTab,
    ToggleSidebar,
    Cancel,
    ReaderNextPage,
    ReaderPreviousPage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Shortcuts(pub BTreeMap<ShortcutAction, Vec<String>>);

impl Default for Shortcuts {
    fn default() -> Self {
        serde_json::from_str(include_str!("../shortcuts.default.json"))
            .expect("default shortcuts are valid JSON")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Combination {
    key: String,
    control: bool,
    meta: bool,
    alt: bool,
    shift: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub control: bool,
    pub meta: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Combination {
    fn parse(value: &str) -> Result<Self, String> {
        let mut parts: Vec<_> = value.split('+').map(str::trim).collect();
        let key = parts.pop().unwrap_or_default().to_ascii_lowercase();
        let named = matches!(
            key.as_str(),
            "tab" | "escape" | "enter" | "space" | "delete" | "backspace" | "pageup" | "pagedown"
        );
        let function = key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=12).contains(&n));
        if !(named || function || key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric()) {
            return Err(format!("Invalid shortcut key: {value}"));
        }
        let mut combination = Self {
            key,
            control: false,
            meta: false,
            alt: false,
            shift: false,
        };
        for part in parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "command" => {
                    if cfg!(target_os = "macos") {
                        &mut combination.meta
                    } else {
                        &mut combination.control
                    }
                }
                "ctrl" | "control" => &mut combination.control,
                "cmd" | "meta" | "super" => &mut combination.meta,
                "alt" | "option" => &mut combination.alt,
                "shift" => &mut combination.shift,
                _ => return Err(format!("Invalid shortcut modifier: {value}")),
            };
            if *flag {
                return Err(format!("Repeated shortcut modifier: {value}"));
            }
            *flag = true;
        }
        if !combination.control
            && !combination.meta
            && !combination.alt
            && combination.key != "escape"
            && combination.key != "pageup"
            && combination.key != "pagedown"
            && !function
        {
            return Err(format!("Shortcut requires Ctrl, Command or Alt: {value}"));
        }
        Ok(combination)
    }
}

impl Shortcuts {
    pub fn validate(&self) -> Result<(), String> {
        let mut used = BTreeSet::new();
        for bindings in self.0.values() {
            for binding in bindings {
                if !used.insert(Combination::parse(binding)?) {
                    return Err(format!("Shortcut is assigned more than once: {binding}"));
                }
            }
        }
        Ok(())
    }

    pub fn action(&self, key: &str, modifiers: Modifiers) -> Option<ShortcutAction> {
        let pressed = Combination {
            key: key.to_ascii_lowercase(),
            control: modifiers.control,
            meta: modifiers.meta,
            alt: modifiers.alt,
            shift: modifiers.shift,
        };
        self.0.iter().find_map(|(action, bindings)| {
            bindings
                .iter()
                .any(|binding| Combination::parse(binding).is_ok_and(|binding| binding == pressed))
                .then_some(*action)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_match_modifiers_exactly() {
        let shortcuts = Shortcuts::default();
        shortcuts.validate().unwrap();
        let modifiers = Modifiers {
            meta: cfg!(target_os = "macos"),
            control: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        assert_eq!(shortcuts.action("s", modifiers), Some(ShortcutAction::Save));
        assert_eq!(
            shortcuts.action(
                "s",
                Modifiers {
                    shift: true,
                    ..modifiers
                }
            ),
            Some(ShortcutAction::SaveAs)
        );
        assert_eq!(shortcuts.action("s", Modifiers::default()), None);
    }

    #[test]
    fn custom_disabled_duplicate_and_invalid_bindings() {
        let mut shortcuts = Shortcuts::default();
        shortcuts
            .0
            .insert(ShortcutAction::Save, vec!["Alt+S".into()]);
        assert_eq!(
            shortcuts.action(
                "s",
                Modifiers {
                    alt: true,
                    ..Default::default()
                }
            ),
            Some(ShortcutAction::Save)
        );
        shortcuts.0.insert(ShortcutAction::Save, vec![]);
        assert_eq!(
            shortcuts.action(
                "s",
                Modifiers {
                    alt: true,
                    ..Default::default()
                }
            ),
            None
        );
        shortcuts
            .0
            .insert(ShortcutAction::Save, vec!["Command+N".into()]);
        assert!(shortcuts.validate().is_err());
        for binding in ["N", "Command+", "Unknown+N", "Command+Shift+Shift+S"] {
            shortcuts
                .0
                .insert(ShortcutAction::Save, vec![binding.into()]);
            assert!(shortcuts.validate().is_err());
        }
    }
}
