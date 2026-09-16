fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "get_runtime_info",
        "list_tasks",
        "create_task",
        "cancel_task",
        "pet_action",
        "set_pet_regions",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("failed to build desktop application manifest");
}
