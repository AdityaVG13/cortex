#!/usr/bin/env node
const fs=require("fs");const path=require("path");const crypto=require("crypto");const https=require("https")
;const{spawnSync:spawnSync}=require("child_process");const PLUGIN_ROOT=process.env.CLAUDE_PLUGIN_ROOT;const PLUGIN_DATA=process.env.CLAUDE_PLUGIN_DATA
;if(!PLUGIN_ROOT||!PLUGIN_DATA){console.error("[cortex-plugin] ERROR: CLAUDE_PLUGIN_ROOT and CLAUDE_PLUGIN_DATA must be set");process.exit(1)}
const PLATFORM=process.platform;const ARCH=process.arch;const PLATFORM_MAP={win32:"windows",darwin:"macos",linux:"linux"};const ARCH_MAP={x64:"x86_64",
arm64:"aarch64"};const mappedPlatform=PLATFORM_MAP[PLATFORM];const mappedArch=ARCH_MAP[ARCH];if(!mappedPlatform||!mappedArch){
console.error(`[cortex-plugin] ERROR: Unsupported platform/arch: ${PLATFORM}/${ARCH}`);process.exit(1)}
const pluginJsonPath=path.join(PLUGIN_ROOT,".claude-plugin","plugin.json");let version="0.6.0";try{
const pluginJson=JSON.parse(fs.readFileSync(pluginJsonPath,"utf8"));version=pluginJson.version||version}catch(e){
console.error(`[cortex-plugin] Warning: Could not read plugin.json, using default version ${version}`)}const archiveExt=PLATFORM==="win32"?"zip":"tar.gz"
;const archiveName=`cortex-v${version}-${mappedPlatform}-${mappedArch}.${archiveExt}`;const assetsDir=path.join(PLUGIN_ROOT,"assets")
;const archivePath=path.join(assetsDir,archiveName);const sha256sumsPath=path.join(assetsDir,"SHA256SUMS")
;const binaryName=PLATFORM==="win32"?"cortex.exe":"cortex";const binDir=path.join(PLUGIN_DATA,"bin");const binaryPath=path.join(binDir,binaryName)
;const manifestPath=path.join(PLUGIN_DATA,"runtime-manifest.json");function isAlreadyExtracted(){if(!fs.existsSync(binaryPath)||!fs.existsSync(manifestPath)){
return false}try{const manifest=JSON.parse(fs.readFileSync(manifestPath,"utf8"))
;return manifest.version===version&&manifest.platform===mappedPlatform&&manifest.arch===mappedArch}catch(e){return false}}
function getExpectedSha256(archiveName){if(!fs.existsSync(sha256sumsPath)){
throw new Error(`SHA256SUMS file not found at ${sha256sumsPath}. Plugin installation may be corrupted.`)}const sums=fs.readFileSync(sha256sumsPath,"utf8")
;for(const line of sums.split("\n")){const match=line.match(/^([a-fA-F0-9]{64})\s+\*?(\S+)$/)
;if(match&&(match[2]===archiveName||match[2].endsWith("/"+archiveName))){return match[1].toLowerCase()}}
throw new Error(`Archive ${archiveName} not found in SHA256SUMS`)}function computeSha256(filePath){const hash=crypto.createHash("sha256")
;const data=fs.readFileSync(filePath);hash.update(data);return hash.digest("hex").toLowerCase()}
const GITHUB_RELEASE_BASE="https://github.com/AdityaVG13/cortex/releases/download";function downloadFile(url,destPath){return new Promise((resolve,reject)=>{
const file=fs.createWriteStream(destPath);const request=reqUrl=>{https.get(reqUrl,{headers:{"User-Agent":"cortex-plugin"}},res=>{
if(res.statusCode>=300&&res.statusCode<400&&res.headers.location){res.resume();request(res.headers.location);return}if(res.statusCode!==200){res.resume()
;reject(new Error(`Download failed: HTTP ${res.statusCode} for ${reqUrl}`));return}res.pipe(file);file.on("finish",()=>{file.close();resolve()})
}).on("error",err=>{fs.unlink(destPath,()=>{});reject(err)})};request(url)})}async function ensureAssets(){const needArchive=!fs.existsSync(archivePath)
;const needSums=!fs.existsSync(sha256sumsPath);if(!needArchive&&!needSums)return;const tag=`v${version}`;fs.mkdirSync(assetsDir,{recursive:true});if(needSums){
const sumsUrl=`${GITHUB_RELEASE_BASE}/${tag}/SHA256SUMS`;console.error(`[cortex-plugin] Downloading SHA256SUMS from release ${tag}...`)
;await downloadFile(sumsUrl,sha256sumsPath)}if(needArchive){const archiveUrl=`${GITHUB_RELEASE_BASE}/${tag}/${archiveName}`
;console.error(`[cortex-plugin] Downloading ${archiveName} from release ${tag}...`);await downloadFile(archiveUrl,archivePath)
;console.error(`[cortex-plugin] Download complete.`)}}function escapePowerShellLiteral(value){return value.replace(/'/g,"''")}
function runCommand(command,args,failureMessage){const result=spawnSync(command,args,{stdio:"inherit",shell:false});if(result.error){throw result.error}
if(typeof result.status==="number"&&result.status!==0){throw new Error(`${failureMessage}: exit ${result.status}`)}}
function extractArchive(archivePath,destDir){fs.mkdirSync(destDir,{recursive:true});if(archivePath.endsWith(".tar.gz")){
runCommand("tar",["xzf",archivePath,"-C",destDir],"Failed to extract tar.gz archive")}else if(archivePath.endsWith(".zip")){if(PLATFORM==="win32"){
const archiveLiteral=escapePowerShellLiteral(archivePath);const destLiteral=escapePowerShellLiteral(destDir);try{
runCommand("powershell",["-NoProfile","-Command",`Expand-Archive -LiteralPath '${archiveLiteral}' -DestinationPath '${destLiteral}' -Force`],"Failed to extract ZIP archive via PowerShell")
}catch(_powershellError){console.error("[cortex-plugin] PowerShell Expand-Archive failed; trying tar fallback");try{
runCommand("tar",["xf",archivePath,"-C",destDir],"Failed to extract ZIP archive via tar fallback")}catch(tarError){
throw new Error(`Failed to extract ZIP via PowerShell and tar fallback. Install Git for Windows (tar) or ensure Expand-Archive works: ${tarError.message}`)}}
}else{runCommand("tar",["xf",archivePath,"-C",destDir],"Failed to extract ZIP archive")}}else{throw new Error(`Unknown archive format: ${archivePath}`)}}
async function main(){if(isAlreadyExtracted()){return}console.error(`[cortex-plugin] First run: extracting daemon binary for ${mappedPlatform}-${mappedArch}`)
;await ensureAssets();if(!fs.existsSync(archivePath)){if(process.env.CORTEX_PLUGIN_DEV==="1"){
console.error(`[cortex-plugin] Dev mode: skipping extraction (archive not found at ${archivePath})`);fs.mkdirSync(binDir,{recursive:true})
;fs.writeFileSync(manifestPath,JSON.stringify({version:version,platform:mappedPlatform,arch:mappedArch,extractedAt:(new Date).toISOString(),devMode:true
},null,2));return}throw new Error(`Archive not found: ${archivePath}. Plugin installation may be incomplete.`)}
const expectedSha256=getExpectedSha256(archiveName);const actualSha256=computeSha256(archivePath);if(expectedSha256!==actualSha256){
console.error(`[cortex-plugin] SHA256 verification failed for ${archiveName}`);console.error(`[cortex-plugin]   Expected: ${expectedSha256}`)
;console.error(`[cortex-plugin]   Got:      ${actualSha256}`)
;console.error(`[cortex-plugin] The archive may be corrupted. Re-install the plugin or verify download.`);process.exit(1)}
console.error(`[cortex-plugin] SHA256 verified: ${actualSha256.slice(0,12)}...`);fs.mkdirSync(binDir,{recursive:true});extractArchive(archivePath,binDir)
;if(!fs.existsSync(binaryPath)){throw new Error(`Binary not found after extraction: ${binaryPath}. Archive may be malformed.`)}if(PLATFORM!=="win32"){try{
fs.chmodSync(binaryPath,493)}catch(e){console.error(`[cortex-plugin] Warning: Could not set executable permission: ${e.message}`)}}
fs.writeFileSync(manifestPath,JSON.stringify({version:version,platform:mappedPlatform,arch:mappedArch,extractedAt:(new Date).toISOString(),sha256:actualSha256
},null,2));console.error(`[cortex-plugin] Daemon binary ready at ${binaryPath}`)}main().catch(e=>{console.error(`[cortex-plugin] ERROR: ${e.message}`)
;process.exit(1)});
