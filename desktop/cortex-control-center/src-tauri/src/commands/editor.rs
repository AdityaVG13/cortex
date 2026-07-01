use crate::editor::{detect_editors as detect_editors_impl, setup_editors as setup_editors_impl, EditorDetection};

#[tauri::command]
pub fn setup_editors(editor_ids: Option<Vec<String>>) -> Result<Vec<EditorDetection>, String> {
    setup_editors_impl(editor_ids)
}

#[tauri::command]
pub fn detect_editors() -> Result<Vec<EditorDetection>, String> {
    detect_editors_impl()
}
