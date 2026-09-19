fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "guide_status",
        "guide_complete",
        "voice_probe",
        "voice_cancel",
        "voice_key_set",
        "model_settings_get",
        "model_settings_save",
        "model_key_set",
        "model_key_delete",
        "chat_config",
        "chat_history",
        "chat_clear",
        "chat_generate",
        "chat_cancel",
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
