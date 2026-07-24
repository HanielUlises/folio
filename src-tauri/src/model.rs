//! Folio data model + persistence. Compatible with the existing
//! `folio-data.json` written by earlier versions.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// A cross-cutting label. A PDF may carry many tags (unlike a single Topic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub added: u64,
    /// Topics this PDF belongs to (a PDF may live in several topics at once).
    #[serde(rename = "topicIds", default)]
    pub topic_ids: Vec<String>,
    /// Legacy single-topic field, migrated into `topic_ids` on load.
    #[serde(rename = "topicId", default, skip_serializing)]
    pub legacy_topic_id: Option<String>,
    #[serde(rename = "tagIds", default)]
    pub tag_ids: Vec<String>,
    #[serde(default = "yes")]
    pub exists: bool,
}

impl PdfEntry {
    pub fn in_topic(&self, id: &str) -> bool {
        self.topic_ids.iter().any(|t| t == id)
    }
}

fn yes() -> bool {
    true
}

/// Highlight rect in normalised page coordinates (0..1 of page width/height),
/// so it stays aligned at any zoom.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HighlightRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfHighlight {
    pub id: String,
    pub page: u32,
    pub rects: Vec<HighlightRect>,
    pub color: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppData {
    #[serde(default)]
    pub topics: Vec<Topic>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub pdfs: Vec<PdfEntry>,
    #[serde(default)]
    pub highlights: HashMap<String, Vec<PdfHighlight>>,
    /// UI theme: "dark" (default) or "light".
    #[serde(default)]
    pub theme: String,
}

impl AppData {
    pub fn data_file() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.folio.pdf")
            .join("folio-data.json")
    }

    pub fn load() -> Self {
        let path = Self::data_file();
        let mut data: AppData = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // Migrate old single-topic entries into the multi-topic list.
        for p in &mut data.pdfs {
            if let Some(id) = p.legacy_topic_id.take() {
                if !p.topic_ids.contains(&id) {
                    p.topic_ids.push(id);
                }
            }
        }
        data
    }

    pub fn save(&self) {
        let path = Self::data_file();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&path, json);
        }
    }

    pub fn topic(&self, id: &str) -> Option<&Topic> {
        self.topics.iter().find(|t| t.id == id)
    }

    pub fn tag(&self, id: &str) -> Option<&Tag> {
        self.tags.iter().find(|t| t.id == id)
    }
}

/// Parse a `#rrggbb` string into an egui colour.
pub fn parse_hex(hex: &str) -> egui::Color32 {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&h[0..2], 16),
            u8::from_str_radix(&h[2..4], 16),
            u8::from_str_radix(&h[4..6], 16),
        ) {
            return egui::Color32::from_rgb(r, g, b);
        }
    }
    egui::Color32::from_rgb(0xd4, 0xa8, 0x43)
}

/// A curated palette for topics/tags.
pub const PALETTE: &[&str] = &[
    "#d4a843", "#6eb5d4", "#7dcf8c", "#d46e6e", "#b57dcf", "#cf9c7d", "#7db8cf",
    "#cfcf7d", "#cf7da8", "#7dcfcf",
];

/// Highlight colours offered in the reader.
pub const HL_COLORS: &[&str] = &["#f9e04b", "#7de87d", "#7dc3f9", "#f97d7d", "#d4a843"];
