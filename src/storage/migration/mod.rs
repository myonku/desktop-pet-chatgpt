mod legacy;

use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::{self, Read},
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use windows::{
    Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    },
    core::PCWSTR,
};

use crate::platform::windows::{LegacyPreferences, local_path};

use super::{CONFIG_LIMIT, file_suffix, invalid, write_new};

const HISTORY_PROVIDER: &str = "legacy-bailong-codex";
const HISTORY_LIMIT: u64 = 64 * 1024 * 1024;
const SESSION_LIMIT: u64 = 16 * 1024 * 1024;
const RECEIPT_VERSION: u32 = 1;
const CONFIG_FILE: &str = "config.toml";

pub enum Source {
    ConfigJson(PathBuf),
    BailongPreferences(LegacyPreferences),
    BailongHistory(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Kind {
    ConfigJson,
    BailongPreferences,
    BailongHistory,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::ConfigJson => "config-json",
            Self::BailongPreferences => "bailong-preferences",
            Self::BailongHistory => "bailong-history",
        }
    }

    fn limit(self) -> u64 {
        match self {
            Self::BailongHistory => HISTORY_LIMIT,
            _ => CONFIG_LIMIT,
        }
    }

    fn destination(self, root: &Path) -> PathBuf {
        match self {
            Self::BailongHistory => root.join("history").join(HISTORY_PROVIDER),
            _ => root.join(CONFIG_FILE),
        }
    }
}

struct Snapshot {
    kind: Kind,
    bytes: Vec<u8>,
    source_dir: Option<PathBuf>,
}

impl Snapshot {
    fn read(source: Source) -> io::Result<Self> {
        let (kind, path) = match source {
            Source::ConfigJson(path) => (Kind::ConfigJson, path),
            Source::BailongHistory(path) => (Kind::BailongHistory, path),
            Source::BailongPreferences(values) => {
                if values.is_empty() {
                    return Err(io::Error::other("未找到可导入的白龙桌宠偏好。"));
                }
                return Ok(Self {
                    kind: Kind::BailongPreferences,
                    bytes: serde_json::to_vec(&values).map_err(io::Error::other)?,
                    source_dir: None,
                });
            }
        };
        let path = local_path(&local_path(&path)?.canonicalize()?)?;
        let source_dir = (kind == Kind::ConfigJson).then(|| path.parent().unwrap().to_owned());
        Ok(Self {
            kind,
            bytes: read_file(&path, kind.limit())?,
            source_dir,
        })
    }

    fn convert(&self, root: &Path, id: &str) -> io::Result<BTreeMap<String, Vec<u8>>> {
        let config = match self.kind {
            Kind::ConfigJson => {
                legacy::config_json(&self.bytes, self.source_dir.as_ref().unwrap(), root)?
            }
            Kind::BailongPreferences => legacy::preferences(&self.bytes)?,
            Kind::BailongHistory => return legacy::history(&self.bytes, id),
        };
        config.validate(false)?;
        let data = config.encode()?;
        let mut document = std::str::from_utf8(&data)
            .map_err(io::Error::other)?
            .parse::<toml_edit::DocumentMut>()
            .map_err(io::Error::other)?;
        // Preserve the bytes and receipt hashes produced before this preference existed.
        if !config.preferences.start_hidden
            && let Some(preferences) = document["preferences"].as_table_mut()
        {
            preferences.remove("start_hidden");
        }
        let mut bytes = format!("# migration_id = {id}\n").into_bytes();
        bytes.extend(document.to_string().into_bytes());
        if bytes.len() as u64 > CONFIG_LIMIT {
            return Err(invalid("转换后的配置超过 64 KiB。"));
        }
        Ok(BTreeMap::from([(CONFIG_FILE.into(), bytes)]))
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    id: String,
    kind: Kind,
    source_sha256: String,
    files: BTreeMap<String, String>,
}

impl Receipt {
    fn verify(&self, path: &Path) -> io::Result<()> {
        match self.kind {
            Kind::BailongHistory => {
                check_dir(path)?;
                let mut count = 0;
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    let expected = name.to_str().and_then(|name| self.files.get(name));
                    verify_file(&entry.path(), SESSION_LIMIT, expected)?;
                    count += 1;
                }
                if count != self.files.len() {
                    return Err(output_changed());
                }
                Ok(())
            }
            _ => verify_file(path, CONFIG_LIMIT, self.files.get(CONFIG_FILE)),
        }
    }
}

pub struct ImportResult {
    pub id: String,
    pub already_applied: bool,
}

pub fn import(profile: &Path, source: Source) -> io::Result<ImportResult> {
    let source = Snapshot::read(source)?;
    let profile = local_path(profile)?;
    fs::create_dir_all(&profile)?;
    let root = local_path(&profile.canonicalize()?)?;
    let journals = journals(&root, true)?;
    let source_sha256 = digest(&source.bytes);
    let mut identity = Sha256::new();
    identity.update(format!(
        "desktop-pet-migration-{RECEIPT_VERSION}\n{}\n{source_sha256}\n",
        source.kind.name()
    ));
    if let Some(directory) = &source.source_dir {
        identity.update(directory.as_os_str().as_encoded_bytes());
    }
    let id = format!("{:x}", identity.finalize());
    let journal = journals.join(&id);
    backup_source(&journals, &journal, &source)?;

    let result = apply(&root, &journal, &source, source_sha256, &id);
    result
        .map(|already_applied| ImportResult {
            id: id.clone(),
            already_applied,
        })
        .map_err(|error| io::Error::new(error.kind(), format!("{error} 原件备份 ID {id}。")))
}

fn backup_source(journals: &Path, journal: &Path, source: &Snapshot) -> io::Result<()> {
    if checked_metadata(journal)?.is_none() {
        let stage = Stage::new(journals)?;
        write_new(&stage.0.join("source.json"), &source.bytes)?;
        move_new(&stage.0, journal)?;
    }
    check_dir(journal)?;
    if read_file(&journal.join("source.json"), source.kind.limit())? != source.bytes {
        return Err(invalid("迁移备份与输入不一致。"));
    }
    Ok(())
}

fn apply(
    root: &Path,
    journal: &Path,
    source: &Snapshot,
    source_sha256: String,
    id: &str,
) -> io::Result<bool> {
    let files = source.convert(root, id)?;
    let expected = Receipt {
        schema_version: RECEIPT_VERSION,
        id: id.into(),
        kind: source.kind,
        source_sha256,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), digest(bytes)))
            .collect(),
    };
    let destination = source.kind.destination(root);
    if source.kind == Kind::BailongHistory {
        ensure_dir(destination.parent().unwrap())?;
    }
    let plan = journal.join("receipt.json");
    let existing = checked_metadata(&destination)?.is_some();
    if checked_metadata(&plan)?.is_some() {
        if read_receipt(&plan)? != expected {
            return Err(invalid("迁移凭据与本次转换不一致。"));
        }
    } else {
        if existing {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "目标已有数据，请保留当前文件或先回滚原迁移。",
            ));
        }
        let bytes = serde_json::to_vec(&expected).map_err(io::Error::other)?;
        if bytes.len() as u64 > HISTORY_LIMIT {
            return Err(invalid("迁移凭据超过大小上限。"));
        }
        let stage = Stage::new(journal)?;
        let staged = stage.0.join("receipt.json");
        write_new(&staged, &bytes)?;
        move_new(&staged, &plan)?;
    }
    if checked_metadata(&journal.join("rolled-back"))?.is_some() {
        return Err(io::Error::other("此迁移已回滚。"));
    }
    let complete = journal.join("applied");
    let committed = checked_metadata(&complete)?.is_some();
    if !existing {
        if committed {
            return Err(output_changed());
        }
        let stage = Stage::new(journal)?;
        for (name, data) in files {
            write_new(&stage.0.join(name), &data)?;
        }
        if source.kind == Kind::BailongHistory {
            move_new(&stage.0, &destination)?;
        } else {
            move_new(&stage.0.join(CONFIG_FILE), &destination)?;
        }
    }
    expected.verify(&destination)?;
    if !committed {
        write_new(&complete, b"")?;
    }
    Ok(existing)
}

pub fn rollback(profile: &Path, id: &str) -> io::Result<bool> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("迁移 ID 必须为 64 位小写十六进制字符。"));
    }
    let root = local_path(&profile.canonicalize()?)?;
    let journal = journals(&root, false)?.join(id);
    check_dir(&journal)?;
    let receipt = read_receipt(&journal.join("receipt.json"))?;
    let source = read_file(&journal.join("source.json"), receipt.kind.limit())?;
    if receipt.schema_version != RECEIPT_VERSION
        || receipt.id != id
        || receipt.source_sha256 != digest(&source)
    {
        return Err(invalid("迁移凭据或原件备份不一致。"));
    }
    let retired = journal.join("rolled-back");
    if checked_metadata(&retired)?.is_some() {
        receipt.verify(&retired)?;
        return Ok(false);
    }
    let destination = receipt.kind.destination(&root);
    if receipt.kind == Kind::BailongHistory {
        check_dir(destination.parent().unwrap())?;
    }
    receipt.verify(&destination)?;
    move_new(&destination, &retired)?;
    // Preserve external edits even if the destination changed between verification and the move.
    if let Err(error) = receipt.verify(&retired) {
        move_new(&retired, &destination).map_err(|_| {
            io::Error::other("回滚期间目标被外部修改。文件保留在迁移备份的 rolled-back 项中。")
        })?;
        return Err(error);
    }
    Ok(true)
}

fn journals(root: &Path, create: bool) -> io::Result<PathBuf> {
    let backups = root.join("backups");
    let migrations = backups.join("migrations");
    for path in [&backups, &migrations] {
        if create {
            ensure_dir(path)?;
        } else {
            check_dir(path)?;
        }
    }
    Ok(migrations)
}

fn verify_file(path: &Path, limit: u64, expected: Option<&String>) -> io::Result<()> {
    let expected = expected.ok_or_else(output_changed)?;
    if &digest(&read_file(path, limit)?) != expected {
        return Err(output_changed());
    }
    Ok(())
}

fn output_changed() -> io::Error {
    io::Error::other("迁移目标已修改、缺失或含额外文件，保留现有数据。")
}

fn read_receipt(path: &Path) -> io::Result<Receipt> {
    decode_json(&read_file(path, HISTORY_LIMIT)?, "迁移凭据")
}

fn decode_json<T: DeserializeOwned>(bytes: &[u8], label: &str) -> io::Result<T> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|error| {
        // Serde errors can echo invalid string fields from a user's history.
        invalid(&format!(
            "{label}的格式无效（第 {} 行，第 {} 列）。",
            error.line(),
            error.column()
        ))
    })
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    if !checked_metadata(path)?.is_some_and(|info| info.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "未找到所需的普通文件。",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("文件超过迁移大小上限。"));
    }
    Ok(bytes)
}

fn checked_metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(info) if info.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 => {
            Err(invalid("迁移目录或目标包含重解析点。"))
        }
        Ok(info) => Ok(Some(info)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn check_dir(path: &Path) -> io::Result<()> {
    if !checked_metadata(path)?.is_some_and(|info| info.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "未找到所需的迁移目录。",
        ));
    }
    Ok(())
}

fn ensure_dir(path: &Path) -> io::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => check_dir(path),
        Err(error) => Err(error),
    }
}

fn move_new(source: &Path, destination: &Path) -> io::Result<()> {
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let from = wide(source);
    let to = wide(destination);
    // Same-volume publication, without MOVEFILE_REPLACE_EXISTING or COPY_ALLOWED.
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(io::Error::other)
}

struct Stage(PathBuf);

impl Stage {
    fn new(parent: &Path) -> io::Result<Self> {
        let path = parent.join(format!(".migration-{}", file_suffix()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
