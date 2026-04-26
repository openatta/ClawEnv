//! Mouse MCP tools. Absolute logical pixels; origin is the primary
//! display's top-left corner. Coordinate convention matches enigo's
//! `Coordinate::Abs`.

use async_trait::async_trait;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Mouse, Settings};
use serde::Deserialize;
use serde_json::{json, Value};

use super::registry::{ToolError, ToolHandler, ToolSpec};

fn new_enigo() -> Result<Enigo, ToolError> {
    Enigo::new(&Settings::default())
        .map_err(|e| ToolError::Internal(format!("enigo init: {e}")))
}

fn parse_button(s: &str) -> Result<Button, ToolError> {
    Ok(match s {
        "left" => Button::Left,
        "right" => Button::Right,
        "middle" => Button::Middle,
        other => return Err(ToolError::InvalidArgument(format!("unknown button '{other}'"))),
    })
}

pub struct MouseMove;

#[async_trait]
impl ToolHandler for MouseMove {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_mouse_move".into(),
            description: "Move cursor to absolute (x,y) logical pixels.".into(),
            input_schema: json!({
                "type": "object",
                "required": ["x","y"],
                "properties": {"x":{"type":"integer"},"y":{"type":"integer"}}
            }),
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args { x: i32, y: i32 }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut en = new_enigo()?;
            en.move_mouse(a.x, a.y, Coordinate::Abs)
                .map_err(|e| ToolError::Internal(format!("move: {e}")))?;
            let (rx, ry) = en
                .location()
                .map_err(|e| ToolError::Internal(format!("location: {e}")))?;
            Ok(json!({"ok": true, "x": rx, "y": ry}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

pub struct MouseClick;

#[async_trait]
impl ToolHandler for MouseClick {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_mouse_click".into(),
            description: "Click a mouse button at the current position.".into(),
            input_schema: json!({
                "type":"object","required":["button"],
                "properties":{
                    "button":{"enum":["left","right","middle"]},
                    "count": {"type":"integer","default":1,"minimum":1,"maximum":5}
                }
            }),
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            button: String,
            #[serde(default = "one")] count: u8,
        }
        fn one() -> u8 { 1 }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        let btn = parse_button(&a.button)?;
        let count = a.count.clamp(1, 5);

        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut en = new_enigo()?;
            for _ in 0..count {
                en.button(btn, Direction::Click)
                    .map_err(|e| ToolError::Internal(format!("click: {e}")))?;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(json!({"ok": true, "button": a.button, "count": count}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

pub struct MouseScroll;

#[async_trait]
impl ToolHandler for MouseScroll {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_mouse_scroll".into(),
            description: "Scroll wheel by (dx, dy) logical lines.".into(),
            input_schema: json!({
                "type":"object",
                "properties":{"dx":{"type":"integer","default":0},"dy":{"type":"integer","default":0}}
            }),
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            #[serde(default)] dx: i32,
            #[serde(default)] dy: i32,
        }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut en = new_enigo()?;
            if a.dx != 0 {
                en.scroll(a.dx, Axis::Horizontal)
                    .map_err(|e| ToolError::Internal(format!("scroll h: {e}")))?;
            }
            if a.dy != 0 {
                en.scroll(a.dy, Axis::Vertical)
                    .map_err(|e| ToolError::Internal(format!("scroll v: {e}")))?;
            }
            Ok(json!({"ok": true, "dx": a.dx, "dy": a.dy}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

pub struct MouseDrag;

#[async_trait]
impl ToolHandler for MouseDrag {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "input_mouse_drag".into(),
            description: "Press button at (from_x,from_y), move to (to_x,to_y), release.".into(),
            input_schema: json!({
                "type":"object",
                "required":["from_x","from_y","to_x","to_y"],
                "properties":{
                    "from_x":{"type":"integer"},"from_y":{"type":"integer"},
                    "to_x":  {"type":"integer"},"to_y":  {"type":"integer"},
                    "button":{"enum":["left","right","middle"],"default":"left"}
                }
            }),
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            from_x: i32, from_y: i32,
            to_x: i32, to_y: i32,
            #[serde(default = "left")] button: String,
        }
        fn left() -> String { "left".into() }
        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        let btn = parse_button(&a.button)?;
        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let mut en = new_enigo()?;
            en.move_mouse(a.from_x, a.from_y, Coordinate::Abs)
                .map_err(|e| ToolError::Internal(format!("move from: {e}")))?;
            en.button(btn, Direction::Press)
                .map_err(|e| ToolError::Internal(format!("btn press: {e}")))?;
            std::thread::sleep(std::time::Duration::from_millis(30));
            en.move_mouse(a.to_x, a.to_y, Coordinate::Abs)
                .map_err(|e| ToolError::Internal(format!("move to: {e}")))?;
            std::thread::sleep(std::time::Duration::from_millis(30));
            en.button(btn, Direction::Release)
                .map_err(|e| ToolError::Internal(format!("btn release: {e}")))?;
            Ok(json!({"ok": true}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}
