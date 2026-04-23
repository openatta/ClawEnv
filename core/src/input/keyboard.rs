//! Keyboard MCP tools — `input_keyboard_type` and `input_keyboard_key`.
//! enigo is not Send on all platforms (macOS CGEvent uses thread-local
//! state) and initialisation is cheap, so every call constructs a fresh
//! Enigo on a `spawn_blocking` worker. No shared state, no `Sync` dance.

use async_trait::async_trait;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde::Deserialize;
use serde_json::{json, Value};

use super::registry::{ToolError, ToolHandler, ToolSpec};

pub struct TypeText;

#[async_trait]
impl ToolHandler for TypeText {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_keyboard_type".into(),
            description: "Type a Unicode string as if on the keyboard.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text":     {"type": "string"},
                    "delay_ms": {"type": "integer", "default": 0, "minimum": 0, "maximum": 500}
                }
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            text: String,
            #[serde(default)]
            delay_ms: u64,
        }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        if a.delay_ms > 500 {
            return Err(ToolError::InvalidArgument("delay_ms must be ≤ 500".into()));
        }

        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut enigo = Enigo::new(&Settings::default())
                .map_err(|e| ToolError::Internal(format!("enigo init: {e}")))?;
            if a.delay_ms == 0 {
                enigo
                    .text(&a.text)
                    .map_err(|e| ToolError::Internal(format!("enigo text: {e}")))?;
            } else {
                let mut tmp = [0u8; 4];
                for ch in a.text.chars() {
                    let s: &str = ch.encode_utf8(&mut tmp);
                    enigo
                        .text(s)
                        .map_err(|e| ToolError::Internal(format!("enigo text: {e}")))?;
                    std::thread::sleep(std::time::Duration::from_millis(a.delay_ms));
                }
            }
            Ok(json!({"ok": true, "chars": a.text.chars().count()}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

pub struct PressKey;

#[async_trait]
impl ToolHandler for PressKey {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_keyboard_key".into(),
            description: "Press a named key with optional modifiers.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["key"],
                "properties": {
                    "key":       {"type": "string"},
                    "modifiers": {"type":"array","items":{"enum":["ctrl","alt","shift","meta"]}},
                    "hold_ms":   {"type":"integer","default":0,"minimum":0,"maximum":5000}
                }
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            key: String,
            #[serde(default)]
            modifiers: Vec<String>,
            #[serde(default)]
            hold_ms: u64,
        }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        let key = parse_key(&a.key)?;
        let mods: Vec<Key> = a
            .modifiers
            .iter()
            .map(|m| parse_modifier(m))
            .collect::<Result<Vec<_>, _>>()?;

        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut enigo = Enigo::new(&Settings::default())
                .map_err(|e| ToolError::Internal(format!("enigo init: {e}")))?;
            for m in &mods {
                enigo
                    .key(*m, Direction::Press)
                    .map_err(|e| ToolError::Internal(format!("mod press {m:?}: {e}")))?;
            }
            if a.hold_ms == 0 {
                enigo
                    .key(key, Direction::Click)
                    .map_err(|e| ToolError::Internal(format!("key click {key:?}: {e}")))?;
            } else {
                enigo
                    .key(key, Direction::Press)
                    .map_err(|e| ToolError::Internal(format!("key press {key:?}: {e}")))?;
                std::thread::sleep(std::time::Duration::from_millis(a.hold_ms));
                enigo
                    .key(key, Direction::Release)
                    .map_err(|e| ToolError::Internal(format!("key release {key:?}: {e}")))?;
            }
            for m in mods.iter().rev() {
                enigo
                    .key(*m, Direction::Release)
                    .map_err(|e| ToolError::Internal(format!("mod release {m:?}: {e}")))?;
            }
            Ok(json!({"ok": true}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

fn parse_modifier(s: &str) -> Result<Key, ToolError> {
    Ok(match s {
        "ctrl" => Key::Control,
        "alt" => Key::Alt,
        "shift" => Key::Shift,
        "meta" => Key::Meta,
        other => return Err(ToolError::InvalidArgument(format!("unknown modifier '{other}'"))),
    })
}

fn parse_key(s: &str) -> Result<Key, ToolError> {
    Ok(match s {
        "Enter" | "enter" | "Return" => Key::Return,
        "Escape" | "Esc" => Key::Escape,
        "Tab" => Key::Tab,
        "Space" => Key::Space,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "ArrowLeft" | "Left" => Key::LeftArrow,
        "ArrowRight" | "Right" => Key::RightArrow,
        "ArrowUp" | "Up" => Key::UpArrow,
        "ArrowDown" | "Down" => Key::DownArrow,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        f if f.starts_with('F') && f.len() > 1 => {
            let n: u32 = f[1..]
                .parse()
                .map_err(|_| ToolError::InvalidArgument(format!("bad function key: {f}")))?;
            match n {
                1 => Key::F1, 2 => Key::F2, 3 => Key::F3, 4 => Key::F4,
                5 => Key::F5, 6 => Key::F6, 7 => Key::F7, 8 => Key::F8,
                9 => Key::F9, 10 => Key::F10, 11 => Key::F11, 12 => Key::F12,
                _ => return Err(ToolError::InvalidArgument(format!("F-key out of range: {f}"))),
            }
        }
        s if s.chars().count() == 1 => {
            Key::Unicode(s.chars().next().expect("len==1 guaranteed"))
        }
        other => return Err(ToolError::InvalidArgument(format!("unknown key '{other}'"))),
    })
}
