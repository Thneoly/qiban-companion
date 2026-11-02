//! Native Live2D model import (A4 / TUWE-14): the folder picker and every
//! file read happen in Rust, so the WebView never opens a file input. That
//! removes WebView2's "上传到此站点？" upload bubble at the source — the
//! panel's local-only promise holds structurally, not just in copy — and
//! keeps working even when the webview file channel is wedged (A6).
//!
//! File-level policy stays in TS (avatarStorage.importModel): the command
//! only mirrors the folder caps (fail fast before an oversized IPC) and
//! re-validates the names it generates, because the TS side re-parses them
//! as untrusted relative paths.
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::ipc::Response;

/// Folder-level caps shared with avatarStorage.importModel.
const MAX_FILES: usize = 256;
const MAX_TOTAL_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedFile {
    path: String,
    /// Byte range of this file inside the data section (the bytes that
    /// follow the manifest), so the manifest's own length never feeds back
    /// into its content.
    offset: usize,
    len: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickedManifest {
    cancelled: bool,
    files: Vec<PickedFile>,
}

/// [4-byte BE manifest length][manifest UTF-8 JSON][file bytes...]; the TS
/// mirror is decodePickedFolder in avatarStorage.ts.
fn payload(manifest: &PickedManifest, blobs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(manifest).map_err(|_| "模型清单序列化失败")?;
    let mut body = Vec::with_capacity(4 + json.len() + blobs.iter().map(Vec::len).sum::<usize>());
    body.extend_from_slice(&(json.len() as u32).to_be_bytes());
    body.extend_from_slice(&json);
    for blob in blobs {
        body.extend_from_slice(blob);
    }
    Ok(body)
}

/// Same contract as the TS localPath guard, applied to names we generate:
/// forward-slash relative paths without `\ : % ? #`, control characters or
/// `.`/`..` segments. The user gets the offending name, not a mystery error.
fn relative_key(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "路径不在所选文件夹内".to_string())?;
    let mut key = String::new();
    for component in relative.iter() {
        let name = component
            .to_str()
            .ok_or_else(|| "文件名需要是常规字符".to_string())?;
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['\\', ':', '%', '?', '#'])
            || name.chars().any(|c| (c as u32) < 0x20)
        {
            return Err(format!("不支持的文件名：{name}"));
        }
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(name);
    }
    Ok(key)
}

/// Depth-first collection of regular files. Links and other reparse points
/// are never followed: a model folder cannot smuggle files from outside the
/// folder the user picked.
fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut dirs = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|error| format!("无法读取文件夹：{error}"))?;
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        paths.sort();
        for path in paths {
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if crate::local_search::is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                dirs.push(path);
            } else {
                if files.len() >= MAX_FILES {
                    return Err(format!("最多 {MAX_FILES} 个文件"));
                }
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Scan and bundle one picked folder into an IPC payload.
fn bundle_folder(root: &Path, max_total_bytes: u64) -> Result<Vec<u8>, String> {
    let files = collect_files(root)?;
    let mut entries = Vec::with_capacity(files.len());
    let mut blobs = Vec::with_capacity(files.len());
    let mut total = 0u64;
    for path in files {
        let bytes = std::fs::read(&path).map_err(|error| format!("读取文件失败：{error}"))?;
        total += bytes.len() as u64;
        if total > max_total_bytes {
            return Err("模型文件夹总大小超过 80 MB".to_string());
        }
        entries.push(PickedFile {
            path: relative_key(root, &path)?,
            offset: 0, // finalized below, once all lengths are known
            len: bytes.len(),
        });
        blobs.push(bytes);
    }
    let mut offset = 0usize;
    for (entry, blob) in entries.iter_mut().zip(blobs.iter()) {
        entry.offset = offset;
        offset += blob.len();
    }
    payload(
        &PickedManifest {
            cancelled: false,
            files: entries,
        },
        &blobs,
    )
}

#[tauri::command]
pub async fn live2d_pick_folder(window: tauri::Window) -> Result<Response, String> {
    #[cfg(windows)]
    let parent = window.hwnd().map_err(|_| "窗口不可用")?.0 as isize;
    #[cfg(not(windows))]
    let parent = 0;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let picked = crate::local_search::choose_folder(parent, "选择 Live2D 模型文件夹")
            .map_err(|_| "文件夹选择失败".to_string())?;
        match picked {
            Some(root) => bundle_folder(&root, MAX_TOTAL_BYTES).map(Some),
            None => Ok(None),
        }
    })
    .await
    .map_err(|_| "文件夹选择失败".to_string())?;
    match picked? {
        Some(body) => Ok(Response::new(body)),
        None => Ok(Response::new(payload(
            &PickedManifest {
                cancelled: true,
                files: Vec::new(),
            },
            &[],
        )?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qiban-live2d-import-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("model")).unwrap();
        std::fs::write(root.join("model/pet.model3.json"), b"{\"Version\":3}").unwrap();
        std::fs::write(root.join("model/body.moc3"), b"MOC3").unwrap();
        std::fs::write(root.join("model/texture.png"), [1u8, 2, 3, 4]).unwrap();
        std::fs::write(root.join("readme.txt"), b"unreferenced").unwrap();
        root
    }

    fn decode(body: &[u8]) -> (serde_json::Value, Vec<u8>) {
        let length = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
        let manifest: serde_json::Value = serde_json::from_slice(&body[4..4 + length]).unwrap();
        (manifest, body[4 + length..].to_vec())
    }

    #[test]
    fn bundles_nested_files_with_sorted_forward_slash_paths() {
        let root = tree("happy");
        let body = bundle_folder(&root, MAX_TOTAL_BYTES).unwrap();
        let (manifest, data) = decode(&body);
        let files = manifest["files"].as_array().unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(
            paths,
            [
                "model/body.moc3",
                "model/pet.model3.json",
                "model/texture.png",
                "readme.txt"
            ]
        );
        assert_eq!(manifest["cancelled"], serde_json::Value::Bool(false));
        // Offsets count from the data section start; each range must replay
        // the exact on-disk bytes.
        for file in files {
            let offset = file["offset"].as_u64().unwrap() as usize;
            let len = file["len"].as_u64().unwrap() as usize;
            let name = file["path"].as_str().unwrap().replace('/', "\\");
            assert_eq!(
                &data[offset..offset + len],
                &std::fs::read(root.join(name)).unwrap()
            );
        }
        // The data section is exactly the declared ranges, packed in order.
        let total: u64 = files.iter().map(|f| f["len"].as_u64().unwrap()).sum();
        assert_eq!(total as usize, data.len());
    }

    #[test]
    fn rejects_oversized_folders_and_unsupported_names() {
        let root = tree("limits");
        let error = bundle_folder(&root, 16).unwrap_err();
        assert_eq!(error, "模型文件夹总大小超过 80 MB");
        assert!(bundle_folder(&root, MAX_TOTAL_BYTES).is_ok());

        let wide = tree("names");
        std::fs::write(wide.join("bad%name.txt"), b"x").unwrap();
        assert_eq!(
            bundle_folder(&wide, MAX_TOTAL_BYTES).unwrap_err(),
            "不支持的文件名：bad%name.txt"
        );

        let many = tree("count");
        let existing = collect_files(&many).unwrap().len();
        for index in 0..(MAX_FILES - existing) {
            std::fs::write(many.join(format!("f{index:03}.txt")), b"x").unwrap();
        }
        assert_eq!(collect_files(&many).unwrap().len(), MAX_FILES);
        assert!(bundle_folder(&many, MAX_TOTAL_BYTES).is_ok());
        std::fs::write(many.join("one-more.txt"), b"x").unwrap();
        assert_eq!(
            bundle_folder(&many, MAX_TOTAL_BYTES).unwrap_err(),
            "最多 256 个文件"
        );
        std::fs::remove_dir_all(&many).unwrap();
    }

    #[test]
    fn cancelled_manifest_is_still_a_valid_payload() {
        let body = payload(
            &PickedManifest {
                cancelled: true,
                files: Vec::new(),
            },
            &[],
        )
        .unwrap();
        let (manifest, data) = decode(&body);
        assert_eq!(manifest["cancelled"], serde_json::Value::Bool(true));
        assert!(manifest["files"].as_array().unwrap().is_empty());
        assert!(data.is_empty());
    }
}
