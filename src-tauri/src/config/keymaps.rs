//! `[keymaps]` config tree. One struct per logical UI group; leaves are
//! typed `Binding { modifiers, key }` inline tables. Nested subgroups
//! (e.g. `keymaps.palette.models`) are their own collision scope —
//! bindings only clash with siblings under the same parent struct.
//!
//! No runtime rebinding: users edit the config file and restart the
//! daemon. `get_keymaps` serves this tree to the webview at boot.

use std::fmt;

use garde::Validate;
use merge::Merge;
use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use super::merge_strategies::overwrite_some;
use super::validations::validate_unique_modifiers;

/// Modifier keys that combine with a `Key` to form a `Binding`. Order
/// in the TOML `modifiers = [...]` list is irrelevant — the
/// `Deserialize` impl on `Binding` canonicalises to ascending order so
/// equality + hashing are independent of source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Alt,
    Ctrl,
    Meta,
    Shift,
}

/// Non-printable / multi-char named keys. Wire form is the DOM
/// `KeyboardEvent.key.toLowerCase()` value so the config TOML matches
/// what the browser emits at runtime — `ArrowUp` → `arrowup`, `PageUp`
/// → `pageup`, `F1` → `f1`. Single-character glyphs (letters, digits,
/// punctuation including shifted symbols `?`, `{`, `|`, `"`, …)
/// deserialize as `Key::Char(c)` under the produced-char rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NamedKey {
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl NamedKey {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Enter => "enter",
            Self::Escape => "escape",
            Self::Tab => "tab",
            Self::Space => "space",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Insert => "insert",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "pageup",
            Self::PageDown => "pagedown",
            Self::ArrowUp => "arrowup",
            Self::ArrowDown => "arrowdown",
            Self::ArrowLeft => "arrowleft",
            Self::ArrowRight => "arrowright",
            Self::F1 => "f1",
            Self::F2 => "f2",
            Self::F3 => "f3",
            Self::F4 => "f4",
            Self::F5 => "f5",
            Self::F6 => "f6",
            Self::F7 => "f7",
            Self::F8 => "f8",
            Self::F9 => "f9",
            Self::F10 => "f10",
            Self::F11 => "f11",
            Self::F12 => "f12",
        }
    }

    fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "enter" => Self::Enter,
            "escape" => Self::Escape,
            "tab" => Self::Tab,
            "space" => Self::Space,
            "backspace" => Self::Backspace,
            "delete" => Self::Delete,
            "insert" => Self::Insert,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            "arrowup" => Self::ArrowUp,
            "arrowdown" => Self::ArrowDown,
            "arrowleft" => Self::ArrowLeft,
            "arrowright" => Self::ArrowRight,
            "f1" => Self::F1,
            "f2" => Self::F2,
            "f3" => Self::F3,
            "f4" => Self::F4,
            "f5" => Self::F5,
            "f6" => Self::F6,
            "f7" => Self::F7,
            "f8" => Self::F8,
            "f9" => Self::F9,
            "f10" => Self::F10,
            "f11" => Self::F11,
            "f12" => Self::F12,
            _ => return None,
        })
    }
}

/// Single keyboard key — either a named token (`enter`, `arrowup`, …)
/// or a single printable character. On the wire always a bare string;
/// the custom `Deserialize` disambiguates on length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Named(NamedKey),
    Char(char),
}

impl Default for Key {
    fn default() -> Self {
        Self::Named(NamedKey::Enter)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(n) => f.write_str(n.as_wire()),
            Self::Char(c) => write!(f, "{c}"),
        }
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Named(n) => serializer.serialize_str(n.as_wire()),
            Self::Char(c) => serializer.serialize_str(&c.to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;

        impl<'de> Visitor<'de> for KeyVisitor {
            type Value = Key;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a single character or a known named key (e.g. 'enter', 'arrowup', 'f1')")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Key, E> {
                if value.is_empty() {
                    return Err(E::custom("key must not be empty"));
                }
                if value != value.to_lowercase() {
                    return Err(E::custom(format!("key must be lowercase (got '{value}')")));
                }
                let mut chars = value.chars();
                let first = chars.next().unwrap();
                if chars.next().is_none() {
                    return Ok(Key::Char(first));
                }
                NamedKey::from_wire(value)
                    .map(Key::Named)
                    .ok_or_else(|| E::custom(format!("unknown named key '{value}'")))
            }
        }

        deserializer.deserialize_str(KeyVisitor)
    }
}

/// Canonical `{ modifiers, key }` tuple. TOML shape:
/// `binding = { modifiers = ["ctrl", "shift"], key = "k" }` or
/// `binding = { key = "enter" }` for unmodified bindings.
///
/// Modifier order in source is irrelevant — `Deserialize` sorts and
/// de-duplicates before returning, so `Eq` / `Hash` ignore source order
/// at comparison time.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq, Hash, Validate)]
pub struct Binding {
    #[garde(custom(validate_unique_modifiers))]
    pub modifiers: Vec<Modifier>,
    #[garde(skip)]
    pub key: Key,
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(default)]
            modifiers: Vec<Modifier>,
            key: Key,
        }
        let Raw { mut modifiers, key } = Raw::deserialize(deserializer)?;
        modifiers.sort();
        Ok(Binding { modifiers, key })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct KeymapsConfig {
    #[garde(dive)]
    pub chat: ChatKeymaps,
    #[garde(dive)]
    pub approvals: ApprovalsKeymaps,
    #[garde(dive)]
    pub composer: ComposerKeymaps,
    #[garde(dive)]
    pub palette: PaletteKeymaps,
    #[garde(dive)]
    pub transcript: TranscriptKeymaps,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ChatKeymaps {
    #[garde(dive)]
    pub submit: Option<Binding>,
    #[garde(dive)]
    pub newline: Option<Binding>,
    /// Cancel the in-flight turn. Sends ACP `CancelNotification` so
    /// the agent stops mid-prompt, clears any pending permissions,
    /// and the composer unlocks. Default: `Ctrl+C` (matching the
    /// shell convention; users override per their layout).
    #[garde(dive)]
    pub cancel_turn: Option<Binding>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ApprovalsKeymaps {
    #[garde(dive)]
    pub allow: Option<Binding>,
    #[garde(dive)]
    pub deny: Option<Binding>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ComposerKeymaps {
    #[garde(dive)]
    pub paste_image: Option<Binding>,
    #[garde(dive)]
    pub tab_completion: Option<Binding>,
    #[garde(dive)]
    pub shift_tab: Option<Binding>,
    #[garde(dive)]
    pub history_up: Option<Binding>,
    #[garde(dive)]
    pub history_down: Option<Binding>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct PaletteKeymaps {
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub open: Option<Binding>,
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub close: Option<Binding>,
    #[garde(dive)]
    pub models: ModelsSubPaletteKeymaps,
    #[garde(dive)]
    pub sessions: SessionsSubPaletteKeymaps,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ModelsSubPaletteKeymaps {
    #[garde(dive)]
    pub focus: Option<Binding>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct SessionsSubPaletteKeymaps {
    #[garde(dive)]
    pub focus: Option<Binding>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct TranscriptKeymaps {}

type ScopeField<'a> = (&'static str, Option<&'a Binding>);
type CollisionScope<'a> = (&'static str, Vec<ScopeField<'a>>);

/// Every binding lives in a collision scope — the parent struct.
/// Subgroups (`palette.models`, `palette.sessions`) are their OWN scopes,
/// NOT merged into the parent palette. Two bindings within the same
/// scope may not share the same `Binding` (modifiers + key); cross-scope
/// collisions are fine.
pub(crate) fn collect_collision_scopes(cfg: &KeymapsConfig) -> Vec<CollisionScope<'_>> {
    vec![
        (
            "chat",
            vec![
                ("submit", cfg.chat.submit.as_ref()),
                ("newline", cfg.chat.newline.as_ref()),
            ],
        ),
        (
            "approvals",
            vec![
                ("allow", cfg.approvals.allow.as_ref()),
                ("deny", cfg.approvals.deny.as_ref()),
            ],
        ),
        (
            "composer",
            vec![
                ("paste_image", cfg.composer.paste_image.as_ref()),
                ("tab_completion", cfg.composer.tab_completion.as_ref()),
                ("shift_tab", cfg.composer.shift_tab.as_ref()),
                ("history_up", cfg.composer.history_up.as_ref()),
                ("history_down", cfg.composer.history_down.as_ref()),
            ],
        ),
        (
            "palette",
            vec![
                ("open", cfg.palette.open.as_ref()),
                ("close", cfg.palette.close.as_ref()),
            ],
        ),
        ("palette.models", vec![("focus", cfg.palette.models.focus.as_ref())]),
        ("palette.sessions", vec![("focus", cfg.palette.sessions.focus.as_ref())]),
        ("transcript", vec![]),
    ]
}

fn binding_display(b: &Binding) -> String {
    let mut out = String::new();
    for m in &b.modifiers {
        match m {
            Modifier::Ctrl => out.push_str("ctrl+"),
            Modifier::Shift => out.push_str("shift+"),
            Modifier::Alt => out.push_str("alt+"),
            Modifier::Meta => out.push_str("meta+"),
        }
    }
    out.push_str(&b.key.to_string());
    out
}

/// Within-scope collision check. Called from `Config::validate()` post-dive.
pub(crate) fn validate_collisions(cfg: &KeymapsConfig) -> anyhow::Result<()> {
    for (scope, fields) in collect_collision_scopes(cfg) {
        let mut seen: std::collections::HashMap<Binding, &str> = std::collections::HashMap::new();
        for (field, binding) in fields {
            let Some(binding) = binding else { continue };
            if let Some(prev) = seen.insert(binding.clone(), field) {
                anyhow::bail!(
                    "keymaps.{scope}: binding '{}' is assigned to both '{prev}' and '{field}' — bindings within the same scope must be unique",
                    binding_display(binding)
                );
            }
        }
    }
    Ok(())
}
