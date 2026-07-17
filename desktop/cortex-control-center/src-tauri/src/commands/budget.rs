use crate::budget::{budget_config_path, read_budget_config_snapshot, save_budget_from_draft, BudgetConfigDraft, BudgetConfigSnapshot};

#[tauri::command]
pub fn read_budget_config() -> Result<BudgetConfigSnapshot, String> {
    let path = budget_config_path()?;
    read_budget_config_snapshot(&path)
}

#[tauri::command]
pub fn save_budget_config(draft: BudgetConfigDraft) -> Result<BudgetConfigSnapshot, String> {
    save_budget_from_draft(draft)
}
