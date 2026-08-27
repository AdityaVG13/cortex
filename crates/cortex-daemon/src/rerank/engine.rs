use std::path::Path;
pub trait Reranker: Send + Sync {
    fn name(&self) -> &'static str;
}
pub struct MiniLmReranker;
impl MiniLmReranker {
    pub fn load(models_dir: &Path) -> Option<Self> {
        super::assets::selected_reranker_assets_exist(models_dir).then_some(Self)
    }
}
impl Reranker for MiniLmReranker {
    fn name(&self) -> &'static str {
        "cross_encoder_minilm_l6_v2"
    }
}
