use std::path::Path;
const MODEL_ENV_KEY: &str = "CORTEX_EMBEDDING_MODEL";
const POOL_ENV_KEY: &str = "CORTEX_EMBED_SESSION_POOL_SIZE";
pub(crate) const TEXT_TRUNCATE_BYTES: usize = 2000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolingStrategy {
    Mean,
    Cls,
    LastToken,
}
impl PoolingStrategy {
    fn as_str(self) -> &'static str {
        match self {
            PoolingStrategy::Mean => "mean",
            PoolingStrategy::Cls => "cls",
            PoolingStrategy::LastToken => "last_token",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum EmbeddingInputKind {
    Query,
    Passage,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct EmbeddingModelAsset {
    pub(crate) file: &'static str,
    pub(crate) url: &'static str,
}
pub(crate) struct EmbeddingModelProfile {
    pub(crate) key: &'static str,
    pub(crate) display_name: &'static str,
    pub(crate) dimension: usize,
    pub(crate) max_input_tokens: usize,
    pub(crate) model_file: &'static str,
    pub(crate) tokenizer_file: &'static str,
    pub(crate) model_url: &'static str,
    pub(crate) tokenizer_url: &'static str,
    pub(crate) auxiliary_files: &'static [EmbeddingModelAsset],
    pub(crate) query_prefix: &'static str,
    pub(crate) passage_prefix: &'static str,
    pub(crate) pooling: PoolingStrategy,
    pub(crate) normalize: bool,
    pub(crate) include_token_type_ids: bool,
}
impl EmbeddingModelProfile {
    fn primary_assets(&self) -> [EmbeddingModelAsset; 2] {
        [
            EmbeddingModelAsset {
                file: self.model_file,
                url: self.model_url,
            },
            EmbeddingModelAsset {
                file: self.tokenizer_file,
                url: self.tokenizer_url,
            },
        ]
    }
    pub(crate) fn missing_assets(&self, models_dir: &Path) -> Vec<EmbeddingModelAsset> {
        let primary = self.primary_assets();
        primary
            .iter()
            .chain(self.auxiliary_files.iter())
            .copied()
            .filter(|asset| !models_dir.join(asset.file).exists())
            .collect()
    }
    pub(crate) fn assets_exist(&self, models_dir: &Path) -> bool {
        self.missing_assets(models_dir).is_empty()
    }
}
const ALL_MINILM_L6_V2: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "all-minilm-l6-v2",
    display_name: "all-MiniLM-L6-v2",
    dimension: 384,
    max_input_tokens: 256,
    model_file: "all-MiniLM-L6-v2.onnx",
    tokenizer_file: "tokenizer.json",
    model_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/onnx/model.onnx",
    tokenizer_url:
        "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix: "",
    passage_prefix: "",
    pooling: PoolingStrategy::Mean,
    normalize: true,
    include_token_type_ids: true,
};
const ALL_MINILM_L12_V2:
EmbeddingModelProfile=EmbeddingModelProfile{key:"all-minilm-l12-v2",display_name:"all-MiniLM-L12-v2",dimension:384,
max_input_tokens:256,model_file:"all-MiniLM-L12-v2.onnx",tokenizer_file:"all-MiniLM-L12-v2-tokenizer.json",model_url:
"https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/onnx/model.onnx",tokenizer_url:
"https://huggingface.co/sentence-transformers/all-MiniLM-L12-v2/resolve/main/tokenizer.json",auxiliary_files:&[],query_prefix:"",
passage_prefix:"",pooling:PoolingStrategy::Mean,normalize:true,include_token_type_ids:true,};
const BGE_BASE_EN_V1_5: EmbeddingModelProfile = EmbeddingModelProfile {
    key: "bge-base-en-v1.5",
    display_name: "bge-base-en-v1.5",
    dimension: 768,
    max_input_tokens: 512,
    model_file: "bge-base-en-v1.5.onnx",
    tokenizer_file: "bge-base-en-v1.5-tokenizer.json",
    model_url: "https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/onnx/model.onnx",
    tokenizer_url: "https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/tokenizer.json",
    auxiliary_files: &[],
    query_prefix: "Represent this sentence for searching relevant passages: ",
    passage_prefix: "",
    pooling: PoolingStrategy::Cls,
    normalize: true,
    include_token_type_ids: true,
};
const QWEN3_EMBEDDING_0_6B:EmbeddingModelProfile=EmbeddingModelProfile{key:"qwen3-embedding-0.6b",
display_name:"Qwen3-Embedding-0.6B",dimension:1024,max_input_tokens:512,model_file:"qwen3-embedding-0.6b/model_uint8.onnx",
tokenizer_file:"qwen3-embedding-0.6b/tokenizer.json",model_url:
"https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/onnx/model_uint8.onnx",tokenizer_url:
"https://huggingface.co/onnx-community/Qwen3-Embedding-0.6B-ONNX/resolve/main/tokenizer.json",auxiliary_files:&[],query_prefix:
"Instruct: Given a web search query, retrieve relevant passages that answer the query\nQuery:",passage_prefix:"",pooling:
PoolingStrategy::LastToken,normalize:true,include_token_type_ids:false,};
const DEFAULT_PROFILE: &EmbeddingModelProfile = &BGE_BASE_EN_V1_5;
#[derive(Clone, Copy, Debug)]
pub struct EmbeddingModelSelection {
    pub key: &'static str,
    pub display_name: &'static str,
    pub dimension: usize,
    pub max_input_tokens: usize,
    pub model_file: &'static str,
    pub tokenizer_file: &'static str,
    pub pooling: &'static str,
}
fn normalize_model_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}
pub(crate) fn resolve_profile() -> &'static EmbeddingModelProfile {
    match std::env::var(MODEL_ENV_KEY) {
        Ok(raw) => match normalize_model_key(&raw).as_str() {
            "all-minilm-l6-v2" | "all-minilm-l6v2" | "minilm-l6" | "minilm-legacy" => {
                &ALL_MINILM_L6_V2
            }
            "all-minilm-l12-v2" | "all-minilm-l12v2" | "minilm-l12" | "minilm-modern"
            | "minilm" => &ALL_MINILM_L12_V2,
            "bge-base-en-v1.5" | "bge-base-en-v15" | "bge-base" | "bge" => &BGE_BASE_EN_V1_5,
            "qwen3-embedding-0.6b" | "qwen3-embedding-06b" | "qwen3-embedding" | "qwen3" => {
                &QWEN3_EMBEDDING_0_6B
            }
            unknown => {
                eprintln!(
                    "[embeddings] Unknown {MODEL_ENV_KEY}='{unknown}', falling back to {}",
                    DEFAULT_PROFILE.key
                );
                DEFAULT_PROFILE
            }
        },
        Err(_) => DEFAULT_PROFILE,
    }
}
pub fn selected_model_selection() -> EmbeddingModelSelection {
    let profile = resolve_profile();
    EmbeddingModelSelection {
        key: profile.key,
        display_name: profile.display_name,
        dimension: profile.dimension,
        max_input_tokens: profile.max_input_tokens,
        model_file: profile.model_file,
        tokenizer_file: profile.tokenizer_file,
        pooling: profile.pooling.as_str(),
    }
}
pub fn selected_model_key() -> &'static str {
    selected_model_selection().key
}
pub fn selected_model_assets_exist(models_dir: &Path) -> bool {
    resolve_profile().assets_exist(models_dir)
}
const DEFAULT_POOL_SIZE: usize = 1;
const MAX_POOL_SIZE: usize = 8;
pub(crate) fn resolved_pool_size() -> usize {
    match std::env::var(POOL_ENV_KEY) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(parsed) => parsed.clamp(1, MAX_POOL_SIZE),
            Err(_) => {
                eprintln!(
                    "[embeddings] Invalid {POOL_ENV_KEY}='{}'; using default {}",
                    raw, DEFAULT_POOL_SIZE
                );
                DEFAULT_POOL_SIZE
            }
        },
        Err(_) => DEFAULT_POOL_SIZE,
    }
}
