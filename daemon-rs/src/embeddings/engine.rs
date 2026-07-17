use super::profiles::{resolve_profile,resolved_pool_size,EmbeddingInputKind,PoolingStrategy,TEXT_TRUNCATE_BYTES};use ort::session
::Session;use ort::value::Tensor;use std::borrow::Cow;use std::path::Path;use tokenizers::Tokenizer;pub struct EmbeddingEngine{
sessions:Vec<std::sync::Mutex<Session>>,next:std::sync::atomic::AtomicUsize,tokenizer:Tokenizer,dimension:usize,max_input_tokens:
usize,model_key:&'static str,query_prefix:&'static str,passage_prefix:&'static str,pooling:PoolingStrategy,normalize:bool,
include_token_type_ids:bool,}impl EmbeddingEngine{pub fn load(models_dir:&Path)->Option<Self>{match Self::try_load(models_dir){Ok(
engine)=>Some(engine),Err(error)=>{eprintln!("[embeddings] Engine load failed: {error}");None}}}fn try_load(models_dir:&Path)->
Result<Self,String>{let profile=resolve_profile();let pool_size=resolved_pool_size();let model_path=models_dir.join(profile.
model_file);let tok_path=models_dir.join(profile.tokenizer_file);let missing_assets=profile.missing_assets(models_dir);if!
missing_assets.is_empty(){let missing=missing_assets.iter().map(|asset|asset.file).collect::<Vec<_>>().join(", ");return Err(
format!("model assets missing ({missing}) at {}",models_dir.display()));}let tokenizer=Tokenizer::from_file(&tok_path).map_err(|
error|format!("failed to load tokenizer {}: {error}",tok_path.display()))?;let mut sessions=Vec::with_capacity(pool_size);for
index in 0..pool_size{let session=Self::build_session(&model_path).map_err(|error|format!("session {} failed: {error}",index+1))?;
sessions.push(std::sync::Mutex::new(session));}eprintln!("[embeddings] Session pool: {pool_size} sessions loaded for {}",profile.
display_name,);Ok(Self{sessions,next:std::sync::atomic::AtomicUsize::new(0),tokenizer,dimension:profile.dimension,max_input_tokens
:profile.max_input_tokens,model_key:profile.key,query_prefix:profile.query_prefix,passage_prefix:profile.passage_prefix,pooling:
profile.pooling,normalize:profile.normalize,include_token_type_ids:profile.include_token_type_ids,})}fn build_session(model_path:&
Path)->Result<Session,String>{let tuned=Session::builder().map_err(|error|format!("session builder init failed: {error}")).
and_then(|builder|builder.with_intra_threads(2).map_err(|error|format!("with_intra_threads(2) failed: {error}"))).and_then(|mut
builder|{builder.commit_from_file(model_path).map_err(|error|format!("commit_from_file (tuned threads) failed for {}: {error}",
model_path.display()))});match tuned{Ok(session)=>Ok(session),Err(tuned_error)=>{let fallback=Session::builder().map_err(|error|
format!("session builder fallback init failed: {error}"))?.commit_from_file(model_path).map_err(|error|format!(
"commit_from_file (fallback threads) failed for {}: {error}",model_path.display()))?;eprintln!(
"[embeddings] Falling back to default ORT session threading after tuned setup failed: {tuned_error}");Ok(fallback)}}}fn
truncate_to_char_boundary(text:&str,max_bytes:usize)->&str{if text.len()<=max_bytes{return text;}let mut end=max_bytes;while end>0
&&!text.is_char_boundary(end){end-=1;}&text[..end]}fn input_text<'a>(&self,text:&'a str,kind:EmbeddingInputKind)->Cow<'a,str>{let
prefix=match kind{EmbeddingInputKind::Query=>self.query_prefix,EmbeddingInputKind::Passage=>self.passage_prefix,};if prefix.
is_empty(){Cow::Borrowed(text)}else{Cow::Owned(format!("{prefix}{text}"))}}fn embed_with_kind(&self,text:&str,kind:
EmbeddingInputKind)->Option<Vec<f32>>{let input=self.input_text(text,kind);let truncated=Self::truncate_to_char_boundary(input.
as_ref(),TEXT_TRUNCATE_BYTES);let encoding=self.tokenizer.encode(truncated,true).ok()?;let ids=encoding.get_ids();let attention=
encoding.get_attention_mask();let type_ids=encoding.get_type_ids();let len=ids.len().min(self.max_input_tokens);if len==0{return
None;}let ids=&ids[..len];let attention=&attention[..len];let type_ids=&type_ids[..len];let shape=vec![1i64,len as i64];let
ids_vec:Vec<i64>=ids.iter().map(|&x|x as i64).collect();let mask_vec:Vec<i64>=attention.iter().map(|&x|x as i64).collect();let
type_vec:Vec<i64>=type_ids.iter().map(|&x|x as i64).collect();let ids_tensor=Tensor::from_array((shape.clone(),ids_vec)).ok()?;let
mask_tensor=Tensor::from_array((shape.clone(),mask_vec)).ok()?;let type_tensor=Tensor::from_array((shape,type_vec)).ok()?;let idx=
self.next.fetch_add(1,std::sync::atomic::Ordering::Relaxed)%self.sessions.len();let mut session=self.sessions[idx].lock().ok()?;
let outputs=if self.include_token_type_ids{session.run(ort::inputs!["input_ids"=>ids_tensor,"attention_mask"=>mask_tensor,
"token_type_ids"=>type_tensor,])}else{session.run(ort::inputs!["input_ids"=>ids_tensor,"attention_mask"=>mask_tensor,])}.ok()?;let
(shape,data)=outputs[0].try_extract_tensor::<f32>().ok()?;let dims:Vec<i64>=shape.iter().copied().collect();if dims.len()!=3||dims
[2]as usize!=self.dimension{eprintln!("[embeddings] Unexpected output shape: {dims:?}");return None;}let seq_len_out=dims[1]as
usize;Self::pool_output(data,self.dimension,seq_len_out,attention,self.pooling,self.normalize)}fn pool_output(data:&[f32],
dimension:usize,seq_len_out:usize,attention:&[u32],pooling:PoolingStrategy,normalize:bool)->Option<Vec<f32>>{if dimension==0||
seq_len_out==0||data.len()<seq_len_out*dimension{return None;}let mut pooled=vec![0.0f32;dimension];match pooling{PoolingStrategy
::Mean=>{let mut mask_sum=0.0f32;let attention_fallback_index=attention.len().saturating_sub(1);for seq_idx in 0..seq_len_out{let
mask_val=attention.get(seq_idx).or_else(||attention.get(attention_fallback_index)).copied().unwrap_or(1)as f32;mask_sum+=mask_val;
let offset=seq_idx*dimension;for dim in 0..dimension{pooled[dim]+=data[offset+dim]*mask_val;}}if mask_sum>0.0{for v in&mut pooled{
*v/=mask_sum;}}}PoolingStrategy::Cls=>{pooled.copy_from_slice(data.get(0..dimension)?);}PoolingStrategy::LastToken=>{let
attention_limit=seq_len_out.min(attention.len());let last_idx=attention.iter().take(attention_limit).rposition(|mask|*mask!=0).
unwrap_or(seq_len_out-1);let offset=last_idx*dimension;pooled.copy_from_slice(data.get(offset..offset+dimension)?);}}if normalize{
let norm:f32=pooled.iter().map(|x|x*x).sum::<f32>().sqrt();if norm>0.0{for v in&mut pooled{*v/=norm;}}}Some(pooled)}pub fn embed(&
self,text:&str)->Option<Vec<f32>>{self.embed_with_kind(text,EmbeddingInputKind::Passage)}pub fn embed_query(&self,text:&str)->
Option<Vec<f32>>{self.embed_with_kind(text,EmbeddingInputKind::Query)}pub async fn embed_async(self:std::sync::Arc<Self>,text:
String)->Option<Vec<f32>>{tokio::task::spawn_blocking(move||self.embed(&text)).await.ok().flatten()}pub async fn embed_query_async
(self:std::sync::Arc<Self>,text:String)->Option<Vec<f32>>{tokio::task::spawn_blocking(move||self.embed_query(&text)).await.ok().
flatten()}pub fn dimension(&self)->usize{self.dimension}pub fn model_key(&self)->&'static str{self.model_key}}
