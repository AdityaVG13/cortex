const RERANK_MODE_ENV:&str="CORTEX_RERANK_MODE";const RERANK_ENABLED_ENV:&str="CORTEX_RERANK_ENABLED";const RERANK_TOP_N_ENV:&str=
"CORTEX_RERANK_TOP_N";const RERANK_FUSION_ALPHA_ENV:&str="CORTEX_RERANK_FUSION_ALPHA";const DEFAULT_TOP_N:usize=24;const MAX_TOP_N
:usize=64;const DEFAULT_FUSION_ALPHA:f64=0.65;#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum RerankMode{Off,Shadow,Primary,}impl
RerankMode{pub fn as_str(self)->&'static str{match self{RerankMode::Off=>"off",RerankMode::Shadow=>"shadow",RerankMode::Primary=>
"primary",}}}#[derive(Clone,Debug)]pub struct RerankConfig{pub mode:RerankMode,pub top_n:usize,pub fusion_alpha:f64,}impl
RerankConfig{pub fn from_env()->Self{let mode=parse_mode_from_env();let top_n=std::env::var(RERANK_TOP_N_ENV).ok().and_then(|raw|
raw.trim().parse::<usize>().ok()).unwrap_or(DEFAULT_TOP_N).clamp(1,MAX_TOP_N);let fusion_alpha=std::env::var(
RERANK_FUSION_ALPHA_ENV).ok().and_then(|raw|raw.trim().parse::<f64>().ok()).filter(|value|value.is_finite()).unwrap_or(
DEFAULT_FUSION_ALPHA).clamp(0.0,1.0);Self{mode,top_n,fusion_alpha}}#[cfg(test)]pub fn off()->Self{Self{mode:RerankMode::Off,top_n:
DEFAULT_TOP_N,fusion_alpha:DEFAULT_FUSION_ALPHA,}}pub fn is_active(&self)->bool{!matches!(self.mode,RerankMode::Off)}pub fn
is_primary(&self)->bool{matches!(self.mode,RerankMode::Primary)}}fn parse_mode_from_env()->RerankMode{if let Ok(raw)=std::env::var
(RERANK_MODE_ENV){match raw.trim().to_ascii_lowercase().as_str(){"off"|"0"|"false"|"disabled"=>return RerankMode::Off,"shadow"|
"trial"|"observe"=>return RerankMode::Shadow,"primary"|"on"|"1"|"true"|"enabled"=>return RerankMode::Primary,unknown=>{eprintln!(
"[rerank] Unknown {RERANK_MODE_ENV}={unknown:?}; using off");return RerankMode::Off;}}}match std::env::var(RERANK_ENABLED_ENV){Ok(
raw)=>match raw.trim().to_ascii_lowercase().as_str(){"1"|"true"|"yes"|"on"|"primary"=>RerankMode::Primary,"shadow"|"trial"=>
RerankMode::Shadow,_=>RerankMode::Off,},Err(_)=>RerankMode::Off,}}
