//! Screen capture MCP tools — `screen_info` and `screen_capture`.
//!
//! Coordinate system: the default is **logical pixels**, matching enigo's
//! mouse API. On a Retina 2x display, a 2880×1800 physical capture is
//! downscaled to 1440×900 before being returned, so the agent can feed a
//! pixel coordinate it found in the image straight into `input_mouse_move`
//! without thinking about scale factors. Agents that *do* want raw
//! physical pixels (for pixel-accurate OCR or template matching) can opt
//! in with `coordinate_system: "physical"`.
//!
//! `region` is interpreted in whichever coordinate system is selected, so
//! the round-trip is internally consistent.

use async_trait::async_trait;
use base64::Engine;
use image::{codecs::jpeg::JpegEncoder, codecs::png::PngEncoder, imageops::FilterType,
            ExtendedColorType, ImageEncoder};
use serde::Deserialize;
use serde_json::{json, Value};
use xcap::Monitor;

use super::registry::{ToolError, ToolHandler, ToolSpec};

pub struct ScreenInfo;

#[async_trait]
impl ToolHandler for ScreenInfo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screen_info".into(),
            description: "List all displays: id, name, width, height, scale_factor, primary.".into(),
            input_schema: json!({"type":"object"}),
        }
    }

    async fn call(&self, _args: Value) -> Result<Value, ToolError> {
        tokio::task::spawn_blocking(|| -> Result<Value, ToolError> {
            let monitors = Monitor::all()
                .map_err(|e| ToolError::PermissionDenied(format!("list monitors: {e}")))?;
            let mut out = Vec::with_capacity(monitors.len());
            for (idx, m) in monitors.iter().enumerate() {
                out.push(json!({
                    "index": idx,
                    "id": m.id().ok(),
                    "name": m.name().ok(),
                    "width": m.width().ok(),
                    "height": m.height().ok(),
                    "scale_factor": m.scale_factor().ok(),
                    "primary": m.is_primary().ok(),
                }));
            }
            Ok(json!({"monitors": out}))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}

pub struct ScreenCapture {
    pub max_dim: u32,
}

#[async_trait]
impl ToolHandler for ScreenCapture {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "screen_capture".into(),
            description: "Capture a display (or region) and return a base64 PNG/JPEG. \
                          Coordinates default to logical pixels (same system as input_mouse_*).".into(),
            input_schema: json!({
                "type":"object",
                "properties":{
                    "display":            {"type":"integer","default":0,"minimum":0},
                    "region":             {"type":"array","items":{"type":"integer"},"minItems":4,"maxItems":4,
                                           "description":"[x,y,w,h] in the selected coordinate_system"},
                    "format":             {"enum":["png","jpeg"],"default":"png"},
                    "return_type":        {"enum":["base64"],"default":"base64"},
                    "coordinate_system":  {"enum":["logical","physical"],"default":"logical",
                                           "description":"'logical' matches input_mouse_* (recommended). \
                                                          'physical' returns raw pixels at display's native resolution."}
                }
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        #[derive(Deserialize)]
        struct Args {
            #[serde(default)] display: usize,
            #[serde(default)] region: Option<[u32; 4]>,
            #[serde(default = "png")] format: String,
            #[serde(default = "b64")] return_type: String,
            #[serde(default = "logical")] coordinate_system: String,
        }
        fn png() -> String { "png".into() }
        fn b64() -> String { "base64".into() }
        fn logical() -> String { "logical".into() }

        let a: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgument(e.to_string()))?;
        if a.return_type != "base64" {
            return Err(ToolError::InvalidArgument("only return_type=base64 is supported".into()));
        }
        let use_logical = match a.coordinate_system.as_str() {
            "logical" => true,
            "physical" => false,
            other => return Err(ToolError::InvalidArgument(format!(
                "unknown coordinate_system '{other}' (want 'logical' or 'physical')"
            ))),
        };
        let max_dim = self.max_dim;

        tokio::task::spawn_blocking(move || -> Result<Value, ToolError> {
            let monitors = Monitor::all()
                .map_err(|e| ToolError::PermissionDenied(format!("list monitors: {e}")))?;
            let mon = monitors.get(a.display).ok_or_else(|| {
                ToolError::InvalidArgument(format!(
                    "display index {} out of range (have {})",
                    a.display, monitors.len()
                ))
            })?;
            let scale = mon.scale_factor().unwrap_or(1.0).max(0.01);
            let img = mon
                .capture_image()
                .map_err(|e| ToolError::Internal(format!("capture: {e}")))?;
            let (phys_w, phys_h) = (img.width(), img.height());

            // Pre-cap on physical pixels so an absurd 8K capture can't
            // first burn CPU on a downscale. max_dim bounds the larger
            // of the two dimensions.
            if phys_w > max_dim || phys_h > max_dim {
                return Err(ToolError::InvalidArgument(format!(
                    "capture {phys_w}x{phys_h} exceeds configured max_dim {max_dim}"
                )));
            }

            // Convert the full frame into the requested coordinate system
            // BEFORE applying region, so callers' region indices always
            // refer to the same coords as the returned image.
            let full = if use_logical && (scale - 1.0).abs() > f32::EPSILON {
                let lw = ((phys_w as f32) / scale).round() as u32;
                let lh = ((phys_h as f32) / scale).round() as u32;
                image::imageops::resize(&img, lw.max(1), lh.max(1), FilterType::Triangle)
            } else {
                img
            };
            let (w, h) = (full.width(), full.height());

            let (rgba, fw, fh) = match a.region {
                Some([rx, ry, rw, rh]) => {
                    if rx.saturating_add(rw) > w || ry.saturating_add(rh) > h {
                        return Err(ToolError::InvalidArgument(format!(
                            "region [{rx},{ry},{rw},{rh}] exceeds image {w}x{h}"
                        )));
                    }
                    let cropped = image::imageops::crop_imm(&full, rx, ry, rw, rh).to_image();
                    (cropped, rw, rh)
                }
                None => (full, w, h),
            };

            let mut buf: Vec<u8> = Vec::new();
            match a.format.as_str() {
                "png" => {
                    PngEncoder::new(&mut buf)
                        .write_image(&rgba, fw, fh, ExtendedColorType::Rgba8)
                        .map_err(|e| ToolError::Internal(format!("png encode: {e}")))?;
                }
                "jpeg" => {
                    // JPEG has no alpha channel; drop it.
                    let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
                    let (rw, rh) = (rgb.width(), rgb.height());
                    JpegEncoder::new_with_quality(&mut buf, 85)
                        .write_image(&rgb, rw, rh, ExtendedColorType::Rgb8)
                        .map_err(|e| ToolError::Internal(format!("jpeg encode: {e}")))?;
                }
                other => {
                    return Err(ToolError::InvalidArgument(format!(
                        "unsupported format '{other}'"
                    )))
                }
            }

            let data = base64::engine::general_purpose::STANDARD.encode(&buf);
            Ok(json!({
                "format": a.format,
                "width": fw,
                "height": fh,
                "bytes": buf.len(),
                "coordinate_system": if use_logical { "logical" } else { "physical" },
                "scale_factor": scale,
                "data": data,
            }))
        })
        .await
        .map_err(|e| ToolError::Internal(format!("join: {e}")))?
    }
}
