use super::paths::{default_home_root,write_secret_file,CortexPaths,CORTEX_DIR_NAME};use super::runtime::{base62_encode_bytes,
fnv1a16,left_pad_base62};use argon2::password_hash::{PasswordHash,PasswordHasher,PasswordVerifier,SaltString};use argon2::{
Algorithm,Argon2,Params,Version};use std::fs;use std::path::PathBuf;use uuid::Uuid;pub fn cortex_dir()->PathBuf{if let Ok(explicit
)=std::env::var("CORTEX_HOME"){if!explicit.trim().is_empty(){return PathBuf::from(explicit);}}default_home_root().join(
CORTEX_DIR_NAME)}pub fn try_generate_token_for(paths:&CortexPaths)->Result<String,String>{let token=Uuid::new_v4().simple().
to_string();try_write_token_for(paths,&token)?;Ok(token)}pub fn try_write_token_for(paths:&CortexPaths,token:&str)->Result<(),
String>{let token_dir=paths.token.parent().unwrap_or(&paths.home);fs::create_dir_all(token_dir).map_err(|e|format!(
"cannot create token directory {}: {e}",token_dir.display()))?;write_secret_file(&paths.token,token.as_bytes()).map_err(|e|format!
("cannot write token file {}: {e}",paths.token.display()))?;Ok(())}pub fn try_generate_token()->Result<String,String>{
try_generate_token_for(&CortexPaths::resolve())}pub fn read_token_from(paths:&CortexPaths)->Option<String>{fs::read_to_string(&
paths.token).ok().map(|v|v.trim().to_string()).filter(|v|!v.is_empty())}pub fn read_token()->Option<String>{read_token_from(&
CortexPaths::resolve())}pub fn generate_ephemeral_token()->String{Uuid::new_v4().simple().to_string()}pub fn generate_ctx_api_key(
)->String{let mut random=Vec::with_capacity(32);random.extend_from_slice(Uuid::new_v4().as_bytes());random.extend_from_slice(Uuid
::new_v4().as_bytes());let mut body=base62_encode_bytes(&random);if body.len()<43{let extra=base62_encode_bytes(Uuid::new_v4().
as_bytes());body.push_str(&extra);}body.truncate(43);let checksum_num=fnv1a16(body.as_bytes());let checksum=left_pad_base62(
checksum_num,3);format!("ctx_{body}{checksum}")}const CTX_KEY_BODY_LEN:usize=43;const CTX_KEY_CHECKSUM_LEN:usize=3;pub fn
verify_ctx_api_key_checksum(candidate:&str)->bool{if!candidate.starts_with("ctx_"){return false;}let payload=&candidate[4..];if
payload.len()!=CTX_KEY_BODY_LEN+CTX_KEY_CHECKSUM_LEN{return false;}if!payload.as_bytes().iter().all(|byte|byte.
is_ascii_alphanumeric()){return false;}let(body,checksum)=payload.split_at(CTX_KEY_BODY_LEN);let expected=left_pad_base62(fnv1a16(
body.as_bytes()),CTX_KEY_CHECKSUM_LEN);constant_time_eq(checksum,expected.as_str())}fn constant_time_eq(a:&str,b:&str)->bool{let a
=a.as_bytes();let b=b.as_bytes();let mut diff=a.len()^b.len();let max_len=a.len().max(b.len());for idx in 0..max_len{let left=a.
get(idx).copied().unwrap_or(0);let right=b.get(idx).copied().unwrap_or(0);diff|=usize::from(left^right);}diff==0}pub fn
hash_api_key_argon2id(api_key:&str)->Result<String,String>{let params=Params::new(64*1024,3,4,None).map_err(|e|e.to_string())?;let
argon2=Argon2::new(Algorithm::Argon2id,Version::V0x13,params);let salt=SaltString::encode_b64(Uuid::new_v4().as_bytes()).map_err(|
e|e.to_string())?;argon2.hash_password(api_key.as_bytes(),&salt).map(|p|p.to_string()).map_err(|e|e.to_string())}pub fn
verify_api_key_argon2id(api_key:&str,hash:&str)->bool{let parsed=match PasswordHash::new(hash){Ok(v)=>v,Err(_)=>return false,};
Argon2::default().verify_password(api_key.as_bytes(),&parsed).is_ok()}
