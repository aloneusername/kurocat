//! Imported picture character packs.
//!
//! A pack is a ZIP with a `character.json` and transparent PNGs (format in
//! `docs/character-pack.md`). Imported packs live in
//! `<app data>/characters/<id>/` and show up in the cat list next to the
//! built-in cats, so menus, settings, OBS and MCP switch to them like any cat.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::cats::sanitize_cat_id;
use crate::config::app_data_dir;

pub const MANIFEST_FILE: &str = "character.json";
pub const STATE_NAMES: [&str; 10] = [
    "idle",
    "blink",
    "typing",
    "typingLeft",
    "typingRight",
    "mouse",
    "mouseClick",
    "privacy",
    "sleep",
    "pet",
];
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 40 * 1024 * 1024;
const MAX_FRAMES_PER_STATE: usize = 60;
const DEFAULT_FPS: f64 = 8.0;

pub const IMPORTED_EVENT: &str = "kuro-character-imported";
pub const ERROR_EVENT: &str = "kuro-character-error";

// ---- character.json as written by creators ----

#[derive(Deserialize)]
#[serde(untagged)]
enum StateSpec {
    One(String),
    Many(Vec<String>),
    Animated {
        frames: Vec<String>,
        #[serde(default)]
        fps: Option<f64>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    format_version: Option<u32>,
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    emoji: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    offset_x: Option<f64>,
    #[serde(default)]
    offset_y: Option<f64>,
    #[serde(default)]
    show_keyboard: Option<bool>,
    #[serde(default)]
    purr: Option<bool>,
    #[serde(default)]
    accent_color: Option<String>,
    #[serde(default)]
    accent_side_color: Option<String>,
    states: BTreeMap<String, StateSpec>,
}

// ---- normalized form sent to the pet / OBS pages ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateFrames {
    pub frames: Vec<String>,
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterInfo {
    /// Changes on every import, so pages reload the pictures.
    pub rev: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub show_keyboard: bool,
    pub purr: bool,
    pub states: BTreeMap<String, StateFrames>,
}

pub struct LoadedCharacter {
    pub id: String,
    pub name: String,
    pub emoji: String,
    pub accent_color: Option<String>,
    pub accent_side_color: Option<String>,
    pub info: CharacterInfo,
}

fn is_hex_color(s: &str) -> bool {
    let hex = s.strip_prefix('#').unwrap_or("");
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// A picture path inside a pack: relative, `/`-separated, ending in `.png`,
/// with no hidden, empty, `.` or `..` segments.
pub fn is_valid_asset_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 200 || !path.to_ascii_lowercase().ends_with(".png") {
        return false;
    }
    path.split('/').all(|seg| {
        !seg.is_empty()
            && !seg.starts_with('.')
            && !seg.chars().any(|c| c == '\\' || c == ':' || c.is_control())
    })
}

fn validate_manifest(m: Manifest) -> Result<(String, Option<String>, CharacterInfo, ManifestStyle), String> {
    if let Some(v) = m.format_version {
        if v != 1 {
            return Err(format!("角色包是第 {v} 版格式，请升级 KuroCat"));
        }
    }
    let name = m.name.trim().to_string();
    if name.is_empty() {
        return Err("character.json 缺少 name".to_string());
    }
    if !m.states.contains_key("idle") {
        return Err("character.json 缺少 idle 待机图".to_string());
    }
    let mut states = BTreeMap::new();
    for (key, spec) in m.states {
        if !STATE_NAMES.contains(&key.as_str()) {
            return Err(format!("不认识的状态名「{key}」"));
        }
        let (frames, fps) = match spec {
            StateSpec::One(f) => (vec![f], None),
            StateSpec::Many(v) => (v, None),
            StateSpec::Animated { frames, fps } => (frames, fps),
        };
        if frames.is_empty() || frames.len() > MAX_FRAMES_PER_STATE {
            return Err(format!("「{key}」要有 1 到 {MAX_FRAMES_PER_STATE} 张图"));
        }
        for f in &frames {
            if !is_valid_asset_path(f) {
                return Err(format!("「{f}」不是 PNG 文件名"));
            }
        }
        let fps = fps.unwrap_or(DEFAULT_FPS);
        if !fps.is_finite() {
            return Err(format!("「{key}」的 fps 不是数字"));
        }
        states.insert(key, StateFrames { frames, fps: fps.clamp(1.0, 30.0) });
    }
    for color in [&m.accent_color, &m.accent_side_color].into_iter().flatten() {
        if !is_hex_color(color) {
            return Err(format!("颜色「{color}」格式不对"));
        }
    }
    let info = CharacterInfo {
        rev: String::new(),
        author: m.author.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()),
        scale: m.scale.filter(|v| v.is_finite()).unwrap_or(1.0).clamp(0.3, 2.0),
        offset_x: m.offset_x.filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-200.0, 200.0),
        offset_y: m.offset_y.filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-200.0, 200.0),
        show_keyboard: m.show_keyboard.unwrap_or(true),
        purr: m.purr.unwrap_or(false),
        states,
    };
    let extra = ManifestStyle {
        emoji: m.emoji.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()),
        accent_color: m.accent_color,
        accent_side_color: m.accent_side_color,
    };
    Ok((name, m.id, info, extra))
}

struct ManifestStyle {
    emoji: Option<String>,
    accent_color: Option<String>,
    accent_side_color: Option<String>,
}

fn referenced_files(info: &CharacterInfo) -> BTreeSet<String> {
    info.states
        .values()
        .flat_map(|s| s.frames.iter().cloned())
        .collect()
}

// ---- storage ----

pub fn characters_dir() -> PathBuf {
    app_data_dir().join("characters")
}

fn character_dir(id: &str) -> PathBuf {
    characters_dir().join(id)
}

pub fn character_exists(id: &str) -> bool {
    let id = sanitize_cat_id(id);
    character_dir(&id).join(MANIFEST_FILE).is_file()
}

fn load_character(id: &str) -> Option<LoadedCharacter> {
    let path = character_dir(id).join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).ok()?;
    let manifest: Manifest = serde_json::from_str(&text).ok()?;
    let (name, _, mut info, extra) = validate_manifest(manifest).ok()?;
    let rev = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default();
    info.rev = rev;
    Some(LoadedCharacter {
        id: id.to_string(),
        name,
        emoji: extra.emoji.unwrap_or_else(|| "🎨".to_string()),
        accent_color: extra.accent_color,
        accent_side_color: extra.accent_side_color,
        info,
    })
}

/// Every valid imported character, sorted by id.
pub fn load_characters() -> Vec<LoadedCharacter> {
    let Ok(entries) = fs::read_dir(characters_dir()) else {
        return vec![];
    };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|name| !name.starts_with('.') && sanitize_cat_id(name) == *name)
        .collect();
    ids.sort();
    ids.iter().filter_map(|id| load_character(id)).collect()
}

pub fn delete_character(id: &str) -> Result<bool, String> {
    let id = sanitize_cat_id(id);
    let dir = character_dir(&id);
    if !dir.join(MANIFEST_FILE).is_file() {
        return Ok(false);
    }
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn read_asset(id: &str, file: &str) -> Option<Vec<u8>> {
    let id = sanitize_cat_id(id);
    if !is_valid_asset_path(file) {
        return None;
    }
    let loaded = load_character(&id)?;
    if !referenced_files(&loaded.info).contains(file) {
        return None;
    }
    fs::read(character_dir(&id).join(file)).ok()
}

// ---- import ----

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

fn read_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let entry = zip
        .by_name(name)
        .map_err(|_| format!("包里缺少「{name}」"))?;
    if entry.size() > limit {
        return Err(format!("「{name}」太大了"));
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取「{name}」失败：{e}"))?;
    if buf.len() as u64 > limit {
        return Err(format!("「{name}」太大了"));
    }
    Ok(buf)
}

/// Finds `character.json` at the ZIP root, or inside a single top folder
/// (what "compress folder" produces). Returns the folder prefix.
fn find_manifest_prefix<R: Read + std::io::Seek>(zip: &zip::ZipArchive<R>) -> Result<String, String> {
    let mut found: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.to_string()))
        .filter(|n| !n.starts_with("__MACOSX/"))
        .filter_map(|n| {
            let parts: Vec<&str> = n.split('/').collect();
            match parts.as_slice() {
                [f] if *f == MANIFEST_FILE => Some(String::new()),
                [dir, f] if *f == MANIFEST_FILE && !dir.starts_with('.') => Some(format!("{dir}/")),
                _ => None,
            }
        })
        .collect();
    found.sort();
    match found.len() {
        0 => Err("角色包里没有 character.json".to_string()),
        1 => Ok(found.remove(0)),
        _ => Err("一个 ZIP 只能放一个角色".to_string()),
    }
}

struct PackContents {
    id: String,
    name: String,
    manifest_bytes: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
}

fn read_pack(zip_path: &Path, taken_ids: &[String]) -> Result<PackContents, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("打不开这个文件：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "这不是有效的 ZIP 文件".to_string())?;
    let prefix = find_manifest_prefix(&zip)?;

    let manifest_bytes = read_entry(&mut zip, &format!("{prefix}{MANIFEST_FILE}"), MAX_MANIFEST_BYTES)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("character.json 写法有误：{e}"))?;
    let (name, raw_id, info, _) = validate_manifest(manifest)?;

    let stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("character");
    let mut id = sanitize_cat_id(raw_id.as_deref().unwrap_or(stem));
    if taken_ids.contains(&id) {
        id = sanitize_cat_id(&format!("{id}-pack"));
    }

    let mut total = manifest_bytes.len() as u64;
    let mut files = Vec::new();
    for rel in referenced_files(&info) {
        let bytes = read_entry(&mut zip, &format!("{prefix}{rel}"), MAX_IMAGE_BYTES)?;
        if !is_png(&bytes) {
            return Err(format!("「{rel}」不是 PNG 图片"));
        }
        total += bytes.len() as u64;
        if total > MAX_PACK_BYTES {
            return Err("图片加起来超过 40 MB".to_string());
        }
        files.push((rel, bytes));
    }
    Ok(PackContents { id, name, manifest_bytes, files })
}

fn install_pack(pack: &PackContents) -> Result<(), String> {
    let root = characters_dir();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = root.join(format!(".import-{}-{nonce}", std::process::id()));
    let result = (|| -> Result<(), String> {
        fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
        fs::write(staging.join(MANIFEST_FILE), &pack.manifest_bytes).map_err(|e| e.to_string())?;
        for (rel, bytes) in &pack.files {
            let dest = staging.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(dest, bytes).map_err(|e| e.to_string())?;
        }
        let target = character_dir(&pack.id);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        fs::rename(&staging, &target).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Ids of built-in and MCP cats an imported pack must not take over.
fn non_character_cat_ids() -> Vec<String> {
    let mut ids: Vec<String> = crate::cats::builtin_cats().into_iter().map(|c| c.id).collect();
    ids.extend(crate::cats::load_custom_cats().into_iter().map(|c| c.id));
    ids
}

/// Installs a pack that was built in a local folder (photo characters).
/// Files the pack does not reference are installed only if named in `extras`.
pub fn install_dir(id: &str, dir: &Path, extras: &[&str]) -> Result<(), String> {
    let manifest_bytes = fs::read(dir.join(MANIFEST_FILE)).map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("character.json 写法有误：{e}"))?;
    let (name, _, info, _) = validate_manifest(manifest)?;
    let mut files = Vec::new();
    for rel in referenced_files(&info) {
        let bytes = fs::read(dir.join(&rel)).map_err(|_| format!("缺少「{rel}」"))?;
        if !is_png(&bytes) {
            return Err(format!("「{rel}」不是 PNG 图片"));
        }
        files.push((rel, bytes));
    }
    for extra in extras {
        if let Ok(bytes) = fs::read(dir.join(extra)) {
            files.push((extra.to_string(), bytes));
        }
    }
    install_pack(&PackContents { id: sanitize_cat_id(id), name, manifest_bytes, files })
}

pub fn import_pack_file(zip_path: &Path) -> Result<(String, String), String> {
    let pack = read_pack(zip_path, &non_character_cat_ids())?;
    install_pack(&pack)?;
    Ok((pack.id, pack.name))
}

pub fn activate_character(app: &AppHandle, id: &str) {
    let mut s = crate::config::load_settings();
    s.cat_breed = id.to_string();
    crate::config::save_settings(&s);
    crate::reload_and_emit_all(app);
}

#[tauri::command]
pub async fn import_character_pack(app: AppHandle) -> Result<Option<String>, String> {
    let result: Result<Option<String>, String> = async {
        let picked =
            crate::file_dialog::pick_file(&app, "选择角色包（ZIP）", "角色包", &["zip"]).await?;
        let Some(path) = picked else {
            return Ok(None);
        };
        let (id, name) = tauri::async_runtime::spawn_blocking(move || import_pack_file(&path))
            .await
            .map_err(|e| e.to_string())??;
        activate_character(&app, &id);
        let _ = app.emit(IMPORTED_EVENT, name);
        Ok(Some(id))
    }
    .await;
    if let Err(msg) = &result {
        let _ = app.emit(ERROR_EVENT, msg.clone());
    }
    result
}

#[tauri::command]
pub fn delete_character_pack(app: AppHandle, id: String) -> Result<bool, String> {
    let removed = delete_character(&id)?;
    if removed {
        let mut s = crate::config::load_settings();
        if s.cat_breed == sanitize_cat_id(&id) {
            s.cat_breed = "black".to_string();
            crate::config::save_settings(&s);
        }
        crate::reload_and_emit_all(&app);
    }
    Ok(removed)
}

fn write_pack_zip(id: &str, dest: &Path) -> Result<(), String> {
    let loaded = load_character(id).ok_or_else(|| "找不到这个角色".to_string())?;
    let dir = character_dir(id);
    let file = fs::File::create(dest).map_err(|e| format!("无法保存到这里：{e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut names = vec![MANIFEST_FILE.to_string()];
    names.extend(referenced_files(&loaded.info));
    for name in names {
        let bytes = fs::read(dir.join(&name)).map_err(|e| e.to_string())?;
        zip.start_file(name.as_str(), options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn export_character_pack(app: AppHandle, id: String) -> Result<bool, String> {
    let id = sanitize_cat_id(&id);
    let result: Result<bool, String> = async {
        if !character_exists(&id) {
            return Err("找不到这个角色".to_string());
        }
        let picked = crate::file_dialog::save_file(
            &app,
            "导出角色包",
            format!("{id}.zip"),
            "角色包",
            &["zip"],
        )
        .await?;
        let Some(dest) = picked else {
            return Ok(false);
        };
        let export_id = id.clone();
        tauri::async_runtime::spawn_blocking(move || write_pack_zip(&export_id, &dest))
            .await
            .map_err(|e| e.to_string())??;
        let _ = app.emit("kuro-character-exported", ());
        Ok(true)
    }
    .await;
    if let Err(msg) = &result {
        let _ = app.emit(ERROR_EVENT, msg.clone());
    }
    result
}

#[tauri::command]
pub fn get_character_asset(id: String, file: String) -> Result<tauri::ipc::Response, String> {
    read_asset(&id, &file)
        .map(tauri::ipc::Response::new)
        .ok_or_else(|| format!("角色图片不存在：{file}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Manifest {
        serde_json::from_str(json).expect("manifest json")
    }

    #[test]
    fn asset_paths_reject_escapes_and_non_png() {
        assert!(is_valid_asset_path("idle.png"));
        assert!(is_valid_asset_path("frames/打字-1.PNG"));
        assert!(!is_valid_asset_path("../idle.png"));
        assert!(!is_valid_asset_path("/abs/idle.png"));
        assert!(!is_valid_asset_path("a\\b.png"));
        assert!(!is_valid_asset_path(".hidden.png"));
        assert!(!is_valid_asset_path("idle.jpg"));
        assert!(!is_valid_asset_path("frames//idle.png"));
    }

    #[test]
    fn states_accept_single_list_and_animated_forms() {
        let (_, _, info, _) = validate_manifest(manifest(
            r#"{"name":"团子","states":{
                "idle":"idle.png",
                "typing":["a.png","b.png"],
                "sleep":{"frames":["z1.png","z2.png"],"fps":99}
            }}"#,
        ))
        .unwrap();
        assert_eq!(info.states["idle"].frames, vec!["idle.png"]);
        assert_eq!(info.states["typing"].frames.len(), 2);
        assert_eq!(info.states["sleep"].fps, 30.0);
        assert!(info.show_keyboard);
        assert!(!info.purr);
    }

    #[test]
    fn manifest_requires_name_idle_and_known_states() {
        assert!(validate_manifest(manifest(r#"{"name":" ","states":{"idle":"a.png"}}"#)).is_err());
        assert!(validate_manifest(manifest(r#"{"name":"a","states":{"blink":"a.png"}}"#)).is_err());
        let err = validate_manifest(manifest(r#"{"name":"a","states":{"idle":"a.png","wave":"b.png"}}"#))
            .err()
            .unwrap();
        assert!(err.contains("wave"));
        assert!(validate_manifest(manifest(
            r#"{"name":"a","accentColor":"blue","states":{"idle":"a.png"}}"#
        ))
        .is_err());
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];

    #[test]
    fn reads_pack_from_single_top_folder_and_avoids_cat_ids() {
        let dir = std::env::temp_dir().join(format!("kurocat-pack-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("my-pack.zip");
        write_zip(
            &zip_path,
            &[
                ("tuanzi/character.json", br#"{"id":"black","name":"Dumpling","states":{"idle":"idle.png"}}"#),
                ("tuanzi/idle.png", PNG),
                ("__MACOSX/tuanzi/._idle.png", b"junk"),
            ],
        );
        let pack = read_pack(&zip_path, &["black".to_string()]).unwrap();
        assert_eq!(pack.id, "black-pack");
        assert_eq!(pack.name, "Dumpling");
        assert_eq!(pack.files.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_missing_or_fake_pictures() {
        let dir = std::env::temp_dir().join(format!("kurocat-pack-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.zip");
        write_zip(&missing, &[("character.json", br#"{"name":"a","states":{"idle":"idle.png"}}"#)]);
        assert!(read_pack(&missing, &[]).err().unwrap().contains("idle.png"));
        let fake = dir.join("fake.zip");
        write_zip(
            &fake,
            &[
                ("character.json", br#"{"name":"a","states":{"idle":"idle.png"}}"#),
                ("idle.png", b"not a png"),
            ],
        );
        assert!(read_pack(&fake, &[]).err().unwrap().contains("PNG"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
