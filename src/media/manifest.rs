use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::{BuildHasher, RandomState},
    io,
    marker::PhantomData,
};

use serde::{Deserialize, Deserializer, de};

use crate::core::{ActionChoice, Behavior, Side};

use super::pack::invalid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "schema_version", deserialize_with = "asset_schema")]
    _schema_version: (),
    pub pack_id: String,
    pub pack_version: String,
    pub color_space: String,
    pub alpha: String,
    pub source_sha256: Vec<String>,
    pub tools: BTreeMap<String, String>,
    engine_api: u32,
    required_features: Vec<String>,
    encoding: String,
    pub pages: Vec<Page>,
    pub clips: Vec<Clip>,
    #[serde(deserialize_with = "unique_map")]
    bindings: BTreeMap<Behavior, Binding>,
    pub resources: Vec<Resource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub size: [u32; 2],
    pub payload: PagePayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PagePayload {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    default: String,
    #[serde(deserialize_with = "unique_map")]
    variants: BTreeMap<String, String>,
}

fn unique_map<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: Deserializer<'de>,
    K: Deserialize<'de> + Ord + fmt::Debug,
    V: Deserialize<'de>,
{
    struct UniqueMap<K, V>(PhantomData<(K, V)>);

    impl<'de, K: Deserialize<'de> + Ord + fmt::Debug, V: Deserialize<'de>> de::Visitor<'de>
        for UniqueMap<K, V>
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object with unique keys")
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry()? {
                match values.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => entry.insert(value),
                    std::collections::btree_map::Entry::Occupied(entry) => {
                        return Err(de::Error::custom(format!(
                            "duplicate binding key {:?}",
                            entry.key()
                        )));
                    }
                };
            }
            Ok(values)
        }
    }

    deserializer.deserialize_map(UniqueMap(PhantomData))
}

pub(super) type ActionBindings = BTreeMap<Behavior, ActionGroup>;

#[derive(Clone)]
pub(super) struct ActionGroup {
    pub default: usize,
    pub variants: BTreeMap<String, usize>,
}

impl ActionGroup {
    pub fn select(
        &self,
        choice: ActionChoice<'_>,
        available: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let default = || available(self.default).then_some(self.default);
        let named = |name| {
            self.variants
                .get(name)
                .copied()
                .filter(|&index| available(index))
        };
        match choice {
            ActionChoice::Default => default(),
            ActionChoice::Variant(name) => named(name).or_else(default),
            ActionChoice::ExactVariant(name) => named(name),
            ActionChoice::Random => {
                let mut candidates = self
                    .variants
                    .values()
                    .copied()
                    .filter(|index| available(*index));
                let count = candidates.clone().count();
                let offset =
                    (count != 0).then(|| RandomState::new().hash_one(count) as usize % count)?;
                candidates.nth(offset)
            }
        }
    }
}

fn asset_schema<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version != 2 {
        return Err(de::Error::custom(format!(
            "unsupported asset pack schema {version}; expected 2"
        )));
    }
    Ok(())
}

impl Manifest {
    pub(super) fn action_bindings(&self) -> io::Result<ActionBindings> {
        semver::Version::parse(&self.pack_version)
            .map_err(|error| invalid(format!("invalid pack version: {error}")))?;
        if self.engine_api != 2 {
            return Err(invalid(format!(
                "unsupported asset engine API {}",
                self.engine_api
            )));
        }
        if let Some(feature) = self.required_features.first() {
            return Err(invalid(format!(
                "unsupported required asset feature {feature}"
            )));
        }
        if self.encoding != "png" {
            return Err(invalid(format!(
                "unsupported asset encoding {}",
                self.encoding
            )));
        }

        let clips: BTreeMap<_, _> = self
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (clip.id.as_str(), index))
            .collect();
        if !self.bindings.contains_key(&Behavior::Idle) {
            return Err(invalid("missing idle binding"));
        }
        let mut actions = ActionBindings::new();
        for (&behavior, binding) in &self.bindings {
            let key = behavior.key();
            let direction = behavior.movement_side().map(|side| match side {
                Side::Left => Direction::Left,
                Side::Right => Direction::Right,
            });
            let mut used = BTreeSet::new();
            let mut variants = BTreeMap::new();
            for (variant, clip) in &binding.variants {
                if !valid_variant(variant) {
                    return Err(invalid(format!(
                        "invalid variant {variant:?} in binding {key}"
                    )));
                }
                let index = *clips.get(clip.as_str()).ok_or_else(|| {
                    invalid(format!("unknown clip {clip} in binding {key}/{variant}"))
                })?;
                if !used.insert(index) {
                    return Err(invalid(format!("duplicate clip {clip} in binding {key}")));
                }
                if let Some(direction) = &direction {
                    let clip = &self.clips[index];
                    if clip.looping
                        || clip.movement_us.is_none()
                        || !clip.directions.contains(direction)
                    {
                        return Err(invalid(format!(
                            "{key}/{variant} needs a non-looping clip with matching direction and movement window"
                        )));
                    }
                }
                variants.insert(variant.clone(), index);
            }
            let default = *variants.get(&binding.default).ok_or_else(|| {
                invalid(format!(
                    "unknown default variant {:?} in binding {key}",
                    binding.default
                ))
            })?;
            actions.insert(behavior, ActionGroup { default, variants });
        }
        if !self.clips[actions[&Behavior::Idle].default].looping {
            return Err(invalid("idle binding must reference a looping clip"));
        }

        Ok(actions)
    }
}

fn valid_variant(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub path: String,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub preview_name: Option<String>,
    pub logical_size: [u32; 2],
    pub looping: bool,
    pub anchor: [f32; 2],
    pub offset: [f32; 2],
    pub scale: f32,
    pub directions: Vec<Direction>,
    pub movement_us: Option<[u64; 2]>,
    pub frames: Vec<Frame>,
}

#[derive(Deserialize, PartialEq, Eq)]
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
