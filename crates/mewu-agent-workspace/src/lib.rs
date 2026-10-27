// SPDX-License-Identifier: MPL-2.0
//! Portable skills and bounded plugin workspaces, independent of any Agent loop.
//! Never executes skill scripts, reads arbitrary files or installs generated code.
mod probe;
pub use probe::{probe,ProbeCase,ProbeAssertion};
use mewu_extension::{ExtensionManifest, ExtensionPackage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::{Read, Write}, path::{Component, Path, PathBuf}};

pub type Result<T> = std::result::Result<T, String>;
const MAX_TEXT: usize = 96 * 1024;
const MAX_PROJECT: usize = 384 * 1024;
const MAX_PROJECTS: usize = 64;
static SKILL_SEED_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn slug(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && !value.starts_with('-') && !value.ends_with('-')
        && !value.contains("--") && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !matches!(value, "con" | "prn" | "aux" | "nul" | "com1" | "com2" | "com3" | "com4" | "com5" | "com6" | "com7" | "com8" | "com9" | "lpt1" | "lpt2" | "lpt3" | "lpt4" | "lpt5" | "lpt6" | "lpt7" | "lpt8" | "lpt9")
}
fn no_link(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path).map_err(|_| "文件不存在或不可读")?;
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 { return Err("不允许链接或重解析点".into()); }
    }
    if m.file_type().is_symlink() { return Err("不允许链接或重解析点".into()); }
    Ok(())
}
fn directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err("无法创建工作目录".into()),
    }
    no_link(path)?;
    if !path.is_dir() { return Err("工作目录无效".into()); }
    Ok(())
}
fn read(path: &Path, max: usize) -> Result<Vec<u8>> {
    no_link(path)?;
    let mut bytes = Vec::new();
    fs::File::open(path).map_err(|_| "无法读取文件")?.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|_| "无法读取文件")?;
    if bytes.len() > max { return Err("文件超过大小上限".into()); }
    Ok(bytes)
}
fn text(path: &Path) -> Result<String> { String::from_utf8(read(path, MAX_TEXT)?).map_err(|_| "文件必须是 UTF-8 文本".into()) }
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() { no_link(path)?; }
    let parent = path.parent().ok_or("文件路径无效")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建工作文件")?;
    tmp.write_all(bytes).map_err(|_| "无法写入工作文件")?;
    tmp.as_file().sync_all().map_err(|_| "无法保存工作文件")?;
    tmp.persist(path).map_err(|_| "无法提交工作文件")?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
}
fn parse_skill(name: &str, value: &str) -> Result<Skill> {
    let value = value.strip_prefix('\u{feff}').unwrap_or(value).replace("\r\n", "\n");
    let front = value.strip_prefix("---\n").and_then(|v| v.split_once("\n---\n").map(|(front, _)| front)).ok_or("SKILL.md 缺少 YAML frontmatter")?;
    if front.len() > 8192 { return Err("技能元数据过长".into()); }
    // YAML is parsed by a bounded YAML parser; unknown portable metadata is kept
    // out of execution policy. In particular allowed-tools never grants access.
    let skill: Skill = serde_saphyr::from_str(front).map_err(|_| "技能 YAML 元数据无效")?;
    if !slug(&skill.name) || skill.name != name || skill.description.trim().is_empty() || skill.description.chars().count() > 1024
        || skill.compatibility.as_ref().is_some_and(|v| v.chars().count() > 500) { return Err("技能名称或描述无效".into()); }
    Ok(skill)
}

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub manifest: ExtensionManifest,
    pub modules: BTreeMap<String, String>,
}
impl Project {
    pub fn validate(&self) -> Result<()> {
        mewu_extension::validate_package(&self.manifest, &self.modules).map_err(|e| e.to_string())?;
        mewu_extension::validate_module_graph(&ExtensionPackage { manifest: self.manifest.clone(), modules: self.modules.clone() }).map_err(|e| e.to_string())
    }
    pub fn archive(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("mewu-plugin.json", options).map_err(|_| "无法打包插件")?;
        zip.write_all(&serde_json::to_vec_pretty(&self.manifest).map_err(|_| "插件清单无效")?).map_err(|_| "无法打包插件")?;
        for (path, content) in &self.modules {
            zip.start_file(path, options).map_err(|_| "无法打包插件")?;
            zip.write_all(content.as_bytes()).map_err(|_| "无法打包插件")?;
        }
        Ok(zip.finish().map_err(|_| "无法打包插件")?.into_inner())
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub project_id: String,
    pub sha256: String,
    pub manifest: ExtensionManifest,
    pub validation: DraftValidation,
}
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all="lowercase")]
pub enum DraftValidation { Untested, Passed, Failed, Unavailable }
#[derive(Debug, Serialize)]
pub struct SkillFiles { pub files: Vec<String>, pub truncated: bool }
pub struct Workspace { root: PathBuf }
impl Workspace {
    pub fn open(data_root: &Path) -> Result<Self> {
        // The storage host owns/canonicalizes data_root. Children are never links.
        let root = data_root.canonicalize().map_err(|_| "数据目录不可用")?;
        directory(&root.join("skills"))?;
        directory(&root.join("plugin-projects"))?;
        Ok(Self { root })
    }
    pub fn seed_skill(&self, name: &str, files: &[(&str, &str)]) -> Result<()> {
        let _gate=SKILL_SEED_GATE.lock().map_err(|_|"无法初始化技能")?;
        if !slug(name) { return Err("技能名称无效".into()); }
        let base = self.root.join("skills").join(name);
        directory(&base)?;
        let receipt=base.join(".mewu-builtin.json");
        let previous:BTreeMap<String,String>=if receipt.exists() {serde_json::from_slice(&read(&receipt,16*1024)?).map_err(|_|"内置技能版本记录无效")?} else {BTreeMap::new()};
        let mut managed=previous.clone();
        for (file, content) in files {
            let path = base.join(file);
            if Path::new(file).components().any(|c| !matches!(c, Component::Normal(_))) || file.contains(['\\', ':']) { return Err("技能文件路径无效".into()); }
            if let Some(parent) = path.parent() {
                for component in parent.strip_prefix(&base).map_err(|_| "路径无效")?.components() {
                    if !matches!(component, Component::Normal(_)) { return Err("路径无效".into()); }
                }
                if parent != base { directory(parent)?; }
            }
            let digest=hash(content.as_bytes());
            // Update only unchanged factory files. User edits and previously
            // unknown files remain untouched, including across app upgrades.
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut output) => {output.write_all(content.as_bytes()).map_err(|_| "无法初始化技能")?;output.sync_all().map_err(|_|"无法保存技能")?;managed.insert((*file).into(),digest);},
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let current=hash(&read(&path,MAX_TEXT)?);
                    if current==digest {managed.insert((*file).into(),digest);}
                    else if previous.get(*file)==Some(&current) {atomic(&path,content.as_bytes())?;managed.insert((*file).into(),digest);}
                },
                Err(_) => return Err("无法初始化技能".into()),
            }
        }
        if managed!=previous {atomic(&receipt,&serde_json::to_vec(&managed).map_err(|_|"内置技能版本记录无效")?)?;}
        Ok(())
    }
    pub fn skills(&self) -> Result<(Vec<Skill>, Vec<String>)> {
        let mut skills = Vec::new(); let mut invalid = Vec::new();
        for entry in fs::read_dir(self.root.join("skills")).map_err(|_| "无法读取技能目录")?.take(257) {
            let entry = entry.map_err(|_| "无法读取技能")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !slug(&name) { continue; }
            if skills.len() + invalid.len() == 256 { return Err("技能数量超过 256，请精简启用目录".into()); }
            let result = no_link(&entry.path()).and_then(|_| text(&entry.path().join("SKILL.md"))).and_then(|v| parse_skill(&name, &v));
            match result { Ok(skill) => skills.push(skill), Err(_) => invalid.push(name) }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name)); invalid.sort(); Ok((skills, invalid))
    }
    pub fn skill_read(&self, name: &str, file: &str) -> Result<String> {
        if !slug(name) || file.is_empty() || file.len() > 240 || file.contains(['\\', ':']) || Path::new(file).components().any(|c| !matches!(c, Component::Normal(_))) { return Err("技能文件路径无效".into()); }
        if file != "SKILL.md" && !["references/", "scripts/", "assets/"].iter().any(|prefix| file.starts_with(prefix)) { return Err("只允许读取技能正文及 references/scripts/assets".into()); }
        let base = self.root.join("skills").join(name);
        no_link(&base)?; parse_skill(name, &text(&base.join("SKILL.md"))?)?;
        let mut path = base;
        for part in Path::new(file).components() { path.push(part); no_link(&path)?; }
        text(&path)
    }
    /// Discover real references instead of asking the model to guess filenames.
    /// A bounded listing is not a limit on the skill's storage or readability.
    pub fn skill_files(&self, name: &str) -> Result<SkillFiles> {
        if !slug(name) { return Err("技能名称无效".into()); }
        let base=self.root.join("skills").join(name); no_link(&base)?;
        parse_skill(name, &text(&base.join("SKILL.md"))?)?;
        let mut files=vec!["SKILL.md".to_owned()]; let mut pending=Vec::new();
        for folder in ["references","scripts","assets"] {
            let path=base.join(folder);
            if path.is_dir() && no_link(&path).is_ok() { pending.push(path); }
        }
        let mut scanned=0; let mut truncated=false;
        'walk: while let Some(dir)=pending.pop() {
            for entry in fs::read_dir(dir).map_err(|_|"无法列出技能文件")? {
                scanned+=1; if scanned>512 {truncated=true;break 'walk;}
                let path=entry.map_err(|_|"无法列出技能文件")?.path();
                if no_link(&path).is_err() {continue;}
                let relative=path.strip_prefix(&base).map_err(|_|"技能路径无效")?;
                if path.is_dir() {
                    if relative.components().count()<8 {pending.push(path);} else {truncated=true;}
                } else if path.is_file() {files.push(relative.to_string_lossy().replace('\\',"/"));}
            }
        }
        files.sort(); Ok(SkillFiles{files,truncated})
    }
    fn project_dir(&self, id: &str, create: bool) -> Result<PathBuf> {
        if !slug(id) { return Err("工程 ID 必须是小写字母、数字和连字符，最长 64 字符".into()); }
        let base = self.root.join("plugin-projects"); no_link(&base)?;
        let path = base.join(id);
        if create && !path.exists() {
            if fs::read_dir(&base).map_err(|_| "无法读取工程目录")?.count() >= MAX_PROJECTS { return Err("插件工程已达 64 个，请整理工作目录".into()); }
            directory(&path)?;
        }
        no_link(&path)?; Ok(path)
    }
    pub fn read_project(&self, id: &str) -> Result<(Project, String)> {
        let path = self.project_dir(id, false)?.join("project.json");
        let bytes = read(&path, MAX_PROJECT)?;
        Ok((serde_json::from_slice(&bytes).map_err(|_| "工程文件无效")?, hash(&bytes)))
    }
    pub fn save(&self, id: &str, project: Project, expected: Option<&str>) -> Result<Draft> {
        let bytes = serde_json::to_vec(&project).map_err(|_| "工程无效")?;
        if bytes.len() > MAX_PROJECT { return Err("工程超过 384 KiB，请拆分或精简插件".into()); }
        project.validate()?;
        let dir = self.project_dir(id, true)?;
        let path = dir.join("project.json");
        let previous = if path.exists() { Some(hash(&read(&path, MAX_PROJECT)?)) } else { None };
        if previous.as_deref() != expected { return Err("工程已变化：先 read，再使用返回的 sha256 作为 expectedSha256".into()); }
        atomic(&path, &bytes)?;
        let sha256=hash(&bytes);
        let validation=self.draft_validation(id,&sha256);
        Ok(Draft { project_id: id.into(), sha256, manifest: project.manifest, validation })
    }
    pub fn drafts(&self) -> Result<Vec<Draft>> {
        let mut drafts = Vec::new();
        for entry in fs::read_dir(self.root.join("plugin-projects")).map_err(|_| "无法读取工程目录")?.take(MAX_PROJECTS) {
            let id = entry.map_err(|_| "无法读取工程")?.file_name().to_string_lossy().into_owned();
            if let Ok((project, sha256)) = self.read_project(&id) {
                let validation=self.draft_validation(&id,&sha256);
                drafts.push(Draft { project_id: id, sha256, manifest: project.manifest, validation });
            }
        }
        drafts.sort_by(|a, b| a.project_id.cmp(&b.project_id)); Ok(drafts)
    }
    pub fn package(&self, id: &str, expected: &str) -> Result<(PathBuf, Vec<u8>)> {
        if self.service_checks(id,expected)?.is_some_and(|v|v["passed"]==false){
            return Err("此工程仍有失败的服务测试；修正后重新 test，不能将失败用例包装为已完成".into());
        }
        let (project, actual) = self.read_project(id)?;
        if actual != expected { return Err("工程已变化，请重新校验".into()); }
        let bytes = project.archive()?;
        // Stable source identity for normal upgrade/CAS; installation keeps its
        // own immutable package bytes, not this editable workspace.
        let path = self.project_dir(id, false)?.join("plugin.zip");
        atomic(&path, &bytes)?; Ok((path, bytes))
    }
    pub fn probe_project(&self,id:&str,expected:&str,cases:&[ProbeCase],configuration:BTreeMap<String,serde_json::Value>,capabilities:serde_json::Value,catalog:serde_json::Value)->Result<serde_json::Value>{
        let (project,actual)=self.read_project(id)?;
        if actual!=expected{return Err("工程已变化，请重新读取后测试".into());}
        let result=probe(&project,cases,configuration,capabilities,catalog)?;
        let receipt=serde_json::json!({"projectSha256":actual,"result":result});
        let bytes=serde_json::to_vec(&receipt).map_err(|_|"测试结果无效")?;
        if bytes.len()>MAX_PROJECT{return Err("测试结果过大，请减少输入和返回内容".into());}
        atomic(&self.project_dir(id,false)?.join("service-checks.json"),&bytes)?;
        Ok(receipt)
    }
    pub fn service_checks(&self,id:&str,expected:&str)->Result<Option<serde_json::Value>>{
        let path=self.project_dir(id,false)?.join("service-checks.json");
        if !path.exists(){return Ok(None);}
        let receipt:serde_json::Value=serde_json::from_slice(&read(&path,MAX_PROJECT)?).map_err(|_|"测试记录无效")?;
        if receipt["projectSha256"]!=expected{return Ok(None);}
        if !receipt["result"]["passed"].is_boolean(){return Err("测试记录无效".into());}
        Ok(Some(receipt["result"].clone()))
    }
    fn draft_validation(&self,id:&str,expected:&str)->DraftValidation {
        match self.service_checks(id,expected) {
            Ok(None)=>DraftValidation::Untested,
            Ok(Some(value))=>match value["passed"].as_bool(){Some(true)=>DraftValidation::Passed,Some(false)=>DraftValidation::Failed,None=>DraftValidation::Unavailable},
            Err(_)=>DraftValidation::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(code: &str) -> Project {
        serde_json::from_value(serde_json::json!({"manifest":{"schemaVersion":2,"hostApi":2,"runtime":"javascript","id":"local.toolbar-order","name":"Order","description":"Test","version":"0.1.0","license":"MPL-2.0","entry":"main.mjs","permissions":["ui.render"],"tags":[],"dependencies":{}},"modules":{"main.mjs":code}})).unwrap()
    }
    #[test] fn project_roundtrip_cas_archive_and_compile_errors() {
        let root=tempfile::tempdir().unwrap(); let ws=Workspace::open(root.path()).unwrap();
        let first=ws.save("toolbar",project("export function activate(ctx) {}"),None).unwrap();
        assert!(ws.save("toolbar",project("export function activate(ctx) {}"),None).is_err());
        assert!(ws.save("toolbar",project("not valid JavaScript {"),Some(&first.sha256)).is_err());
        assert_eq!(ws.read_project("toolbar").unwrap().1,first.sha256);
        let (path,bytes)=ws.package("toolbar",&first.sha256).unwrap(); assert!(path.ends_with("toolbar/plugin.zip"));
        let mut zip=zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap(); assert!(zip.by_name("mewu-plugin.json").is_ok());
        assert!(ws.package("toolbar","stale").is_err()); assert!(ws.save("../escape",project("export function activate() {}"),None).is_err());
    }
    #[test] fn portable_skill_yaml_progressive_read_and_no_overwrite() {
        let root=tempfile::tempdir().unwrap(); let ws=Workspace::open(root.path()).unwrap();
        let skill="---\nname: make-plugin\ndescription: >-\n  Build a portable\n  plugin\nallowed-tools: Bash Read\nmetadata:\n  author: Example\n---\nBody";
        ws.seed_skill("make-plugin",&[("SKILL.md",skill),("references/api.md","API")]).unwrap();
        assert_eq!(ws.skills().unwrap().0[0].description,"Build a portable plugin");
        assert_eq!(ws.skill_read("make-plugin","references/api.md").unwrap(),"API");
        let files=ws.skill_files("make-plugin").unwrap();
        assert_eq!(files.files,vec!["SKILL.md","references/api.md"]); assert!(!files.truncated);
        let updated=skill.replace("Body","Updated factory body");
        ws.seed_skill("make-plugin",&[("SKILL.md",&updated)]).unwrap(); assert_eq!(ws.skill_read("make-plugin","SKILL.md").unwrap(),updated);
        let customized=skill.replace("Body","User custom workflow");
        fs::write(root.path().join("skills/make-plugin/SKILL.md"),&customized).unwrap();
        ws.seed_skill("make-plugin",&[("SKILL.md",skill)]).unwrap(); assert_eq!(ws.skill_read("make-plugin","SKILL.md").unwrap(),customized);
        for path in ["../secret","/secret","references/../../secret","references/a:stream"] { assert!(ws.skill_read("make-plugin",path).is_err()); }
    }
    #[test] fn malformed_skill_does_not_hide_valid_skills() {
        let root=tempfile::tempdir().unwrap(); let ws=Workspace::open(root.path()).unwrap();
        ws.seed_skill("broken",&[("SKILL.md","---\nname: wrong\ndescription: no\n---\n...")]).unwrap();
        assert_eq!(ws.skills().unwrap().1,vec!["broken"]);
    }
    #[cfg(unix)]
    #[test] fn linked_skill_cannot_escape() {
        let root=tempfile::tempdir().unwrap(); let ws=Workspace::open(root.path()).unwrap(); let outside=tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(),root.path().join("skills/escape")).unwrap();
        assert!(ws.skill_read("escape","SKILL.md").is_err());
        assert!(ws.skill_files("escape").is_err());
    }
}
