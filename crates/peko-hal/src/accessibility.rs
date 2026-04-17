use crate::RgbaBuffer;
use std::process::Command;

/// Captures screen via SurfaceFlinger's screencap binary.
/// Works when the Android framework is running (hybrid mode).
pub struct SurfaceFlingerCapture;

impl SurfaceFlingerCapture {
    /// Capture screen as raw RGBA bytes via screencap -p
    pub fn capture_png() -> anyhow::Result<Vec<u8>> {
        let output = Command::new("screencap")
            .arg("-p")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("screencap failed: {}", stderr);
        }

        Ok(output.stdout)
    }

    /// Capture screen as raw pixels via screencap (no PNG encoding)
    pub fn capture_raw() -> anyhow::Result<RgbaBuffer> {
        let output = Command::new("screencap")
            .output()?;

        if !output.status.success() {
            anyhow::bail!("screencap raw failed");
        }

        // screencap raw format: 4 bytes width (LE) + 4 bytes height (LE) + 4 bytes format + pixel data
        let data = &output.stdout;
        if data.len() < 12 {
            anyhow::bail!("screencap output too small");
        }

        let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let _format = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let pixels = data[12..].to_vec();

        Ok(RgbaBuffer { data: pixels, width, height })
    }

    /// Check if screencap binary is available
    pub fn is_available() -> bool {
        Command::new("which")
            .arg("screencap")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Dumps the UI view hierarchy via uiautomator
pub struct UiHierarchy;

#[derive(Debug, Clone)]
pub struct UiNode {
    pub class: String,
    pub text: String,
    pub content_desc: String,
    pub resource_id: String,
    pub bounds: Option<Bounds>,
    pub clickable: bool,
    pub enabled: bool,
    pub focused: bool,
    pub scrollable: bool,
    pub children: Vec<UiNode>,
}

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    pub fn center(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    pub fn width(&self) -> i32 { self.right - self.left }
    pub fn height(&self) -> i32 { self.bottom - self.top }
}

impl UiHierarchy {
    /// Dump the UI hierarchy as raw XML
    pub fn dump_xml() -> anyhow::Result<String> {
        let output = Command::new("uiautomator")
            .arg("dump")
            .arg("/dev/stdout")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("uiautomator dump failed: {}", stderr);
        }

        let xml = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(xml)
    }

    /// Parse XML into a flat list of UI nodes with bounds
    pub fn dump_flat() -> anyhow::Result<Vec<UiNode>> {
        let xml = Self::dump_xml()?;
        let nodes = Self::parse_nodes_from_xml(&xml);
        Ok(nodes)
    }

    /// Find clickable elements containing specific text
    pub fn find_by_text(text: &str) -> anyhow::Result<Vec<UiNode>> {
        let nodes = Self::dump_flat()?;
        let lower = text.to_lowercase();
        Ok(nodes.into_iter()
            .filter(|n| {
                n.text.to_lowercase().contains(&lower)
                    || n.content_desc.to_lowercase().contains(&lower)
            })
            .collect())
    }

    /// Find element by resource ID
    pub fn find_by_id(resource_id: &str) -> anyhow::Result<Option<UiNode>> {
        let nodes = Self::dump_flat()?;
        Ok(nodes.into_iter().find(|n| n.resource_id == resource_id))
    }

    /// Check if uiautomator is available
    pub fn is_available() -> bool {
        Command::new("which")
            .arg("uiautomator")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn parse_nodes_from_xml(xml: &str) -> Vec<UiNode> {
        let mut nodes = Vec::new();

        // Simple attribute parser — avoids XML crate dependency
        for segment in xml.split("<node ") {
            if !segment.contains("bounds=") { continue; }

            let node = UiNode {
                class: extract_attr(segment, "class").unwrap_or_default(),
                text: extract_attr(segment, "text").unwrap_or_default(),
                content_desc: extract_attr(segment, "content-desc").unwrap_or_default(),
                resource_id: extract_attr(segment, "resource-id").unwrap_or_default(),
                bounds: extract_attr(segment, "bounds").and_then(|b| parse_bounds(&b)),
                clickable: extract_attr(segment, "clickable").as_deref() == Some("true"),
                enabled: extract_attr(segment, "enabled").as_deref() != Some("false"),
                focused: extract_attr(segment, "focused").as_deref() == Some("true"),
                scrollable: extract_attr(segment, "scrollable").as_deref() == Some("true"),
                children: Vec::new(),
            };
            nodes.push(node);
        }

        nodes
    }
}

fn extract_attr(segment: &str, name: &str) -> Option<String> {
    let prefix = format!("{}=\"", name);
    let start = segment.find(&prefix)? + prefix.len();
    // Find the closing quote — skip escaped quotes (&quot;)
    // uiautomator XML uses XML entities, not backslash escapes
    let rest = &segment[start..];
    let end = rest.find('"')? + start;
    let raw = &segment[start..end];
    // Decode common XML entities
    let decoded = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    Some(decoded)
}

fn parse_bounds(bounds_str: &str) -> Option<Bounds> {
    // Format: [left,top][right,bottom]
    let s = bounds_str.replace('[', "").replace(']', ",");
    let parts: Vec<i32> = s.split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();

    if parts.len() == 4 {
        Some(Bounds {
            left: parts[0],
            top: parts[1],
            right: parts[2],
            bottom: parts[3],
        })
    } else {
        None
    }
}
