use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub color_space: String,
    pub alpha: String,
    pub source_sha256: Vec<String>,
    pub tools: BTreeMap<String, String>,
    pub pages: Vec<Page>,
    pub actions: Vec<Action>,
    pub resources: Vec<Resource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub size: [u32; 2],
    pub png: Payload,
    pub rgba8: Option<Payload>,
    pub bc7: Option<Payload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub path: String,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub logical_size: [u32; 2],
    pub looping: bool,
    pub anchor: [f32; 2],
    pub offset: [f32; 2],
    pub scale: f32,
    pub directions: Vec<Direction>,
    pub movement_us: Option<[u64; 2]>,
    pub frames: Vec<Frame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub page: String,
    pub rect: [u32; 4],
    pub duration_us: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub key: String,
    pub client_id: Option<String>,
    pub content: ResourceContent,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceContent {
    Image { page: String },
    Audio { file: Payload },
    Icon { file: Payload },
    Text { file: Payload },
}
