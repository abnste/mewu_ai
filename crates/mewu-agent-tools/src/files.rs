// SPDX-License-Identifier: MPL-2.0
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    sync::{Mutex, OnceLock},
    time::Duration,
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn clipped(value: &str, max: usize) -> (&str, bool) {
    if value.len() <= max {
        return (value, false);
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (&value[..end], true)
}
fn load(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Path is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("File exceeds 32 MiB; use shell streaming/range commands".into());
    }
    Ok(bytes)
}
fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| "File is not UTF-8 text; use the shell with the appropriate decoder".into())
}
fn target(cwd: &Path, value: &str) -> Result<PathBuf> {
    let path = resolve_path(cwd, value)?;
    let path = if path.exists() {
        path.canonicalize().map_err(|e| e.to_string())?
    } else {
        path
    };
    if path.to_str().is_none() || serde_json::to_vec(&path).map_err(|e| e.to_string())?.len() > 8192
    {
        return Err(
            "Resolved path exceeds the UTF-8 metadata budget; use byte-safe shell commands".into(),
        );
    }
    Ok(path)
}
fn commit(
    path: &Path,
    bytes: &[u8],
    expected: Option<&[u8]>,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    check()?;
    let parent = path.parent().ok_or("File has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create parent directory: {e}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    temp.write_all(bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    check()?;
    if let Some(expected) = expected {
        if load(path)? != expected {
            return Err(
                "File changed since this edit was prepared; read it again before editing".into(),
            );
        }
    }
    temp.persist(path)
        .map_err(|e| format!("Cannot commit file: {}", e.error))?;
    Ok(())
}
pub fn execute(
    name: &str,
    arguments: &str,
    cwd: &Path,
    check: &dyn Fn() -> Result<()>,
) -> Result<Value> {
    check()?;
    static MUTATIONS: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = if matches!(name, "write" | "edit") {
        Some(loop {
            check()?;
            match MUTATIONS.get_or_init(|| Mutex::new(())).try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err("File mutation queue is unavailable".into()),
            }
        })
    } else {
        None
    };
    match name {
        "read" => {
            let call: ReadCall = parse(arguments)?;
            let path = target(cwd, &call.path)?;
            let bytes = load(&path)?;
            check()?;
            let text = utf8(&bytes)?
                .strip_prefix('\u{feff}')
                .unwrap_or(utf8(&bytes)?);
            let offset = call.offset.unwrap_or(1);
            let limit = call.limit.unwrap_or(2000);
            if offset == 0 || limit == 0 || limit > 10000 {
                return Err("offset is a 1-based line; limit must be 1–10000".into());
            }
            let lines: Vec<_> = text.split_inclusive('\n').collect();
            if offset > lines.len().max(1) {
                return Err(format!("offset exceeds {} lines", lines.len()));
            }
            let mut content = String::new();
            let mut count = 0;
            for line in lines.iter().skip(offset - 1).take(limit) {
                check()?;
                if content.len() + line.len() > MAX_OUTPUT {
                    if count == 0 {
                        return Err("A single line exceeds 16 KiB; use shell byte-range commands to read it".into());
                    }
                    break;
                }
                content.push_str(line);
                count += 1;
            }
            let next = offset - 1 + count;
            Ok(
                json!({"path":path,"content":content,"offset":offset,"lines":count,"totalLines":lines.len(),"nextOffset":(next<lines.len()).then_some(next+1),"sha256":hash(&bytes),"truncated":next<lines.len()}),
            )
        }
        "write" => {
            let call: WriteCall = parse(arguments)?;
            let path = target(cwd, &call.path)?;
            commit(&path, call.content.as_bytes(), None, check)?;
            Ok(
                json!({"path":path,"bytesWritten":call.content.len(),"sha256":hash(call.content.as_bytes()),"committed":true}),
            )
        }
        "edit" => {
            let call: EditCall = parse(arguments)?;
            if call.edits.is_empty() || call.edits.len() > 128 {
                return Err("edits must contain 1–128 replacements".into());
            }
            let path = target(cwd, &call.path)?;
            let bytes = load(&path)?;
            let raw = utf8(&bytes)?;
            let bom = raw.starts_with('\u{feff}');
            let crlf = raw.contains("\r\n");
            let original = raw
                .strip_prefix('\u{feff}')
                .unwrap_or(raw)
                .replace("\r\n", "\n");
            let mut edits = Vec::new();
            for replacement in call.edits {
                check()?;
                let old = replacement.old_text.replace("\r\n", "\n");
                let new = replacement.new_text.replace("\r\n", "\n");
                if old.is_empty() {
                    return Err("oldText must be nonempty".into());
                }
                let start = original.find(&old).ok_or(
                    "oldText does not match the file; read the actual file and correct the edit",
                )?;
                if original.rfind(&old) != Some(start) {
                    return Err("oldText is not unique; include enough surrounding text".into());
                }
                edits.push((start, start + old.len(), new));
            }
            edits.sort_by_key(|v| v.0);
            if edits.windows(2).any(|w| w[0].1 > w[1].0) {
                return Err(
                    "Edits overlap in the original file; combine them into one replacement".into(),
                );
            }
            let first_line = original[..edits[0].0]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1;
            let mut output = original.clone();
            for (start, end, new) in edits.into_iter().rev() {
                output.replace_range(start..end, &new);
            }
            let diff = similar::TextDiff::from_lines(&original, &output)
                .unified_diff()
                .header(&call.path, &call.path)
                .to_string();
            let mut saved = if crlf {
                output.replace('\n', "\r\n")
            } else {
                output
            };
            if bom {
                saved.insert(0, '\u{feff}');
            }
            commit(&path, saved.as_bytes(), Some(&bytes), check)?;
            let (diff, truncated) = clipped(&diff, MAX_OUTPUT);
            Ok(
                json!({"path":path,"sha256":hash(saved.as_bytes()),"bytesWritten":saved.len(),"committed":true,"diff":diff,"diffTruncated":truncated,"firstChangedLine":first_line}),
            )
        }
        "ls" | "find" | "grep" => search(name, parse(arguments)?, cwd, check),
        _ => Err("Not a filesystem tool".into()),
    }
}
fn search(
    name: &str,
    call: SearchCall,
    cwd: &Path,
    check: &dyn Fn() -> Result<()>,
) -> Result<Value> {
    let root = target(cwd, &call.path)?;
    let limit = call.limit.unwrap_or(1000);
    if limit == 0 || limit > 10000 {
        return Err("limit must be 1–10000".into());
    }
    let matcher = if name == "grep" {
        let p = call.pattern.as_deref().ok_or("grep requires pattern")?;
        Some(
            regex::RegexBuilder::new(&if call.literal {
                regex::escape(p)
            } else {
                p.into()
            })
            .case_insensitive(call.ignore_case)
            .size_limit(2 * 1024 * 1024)
            .build()
            .map_err(|e| format!("Invalid regex: {e}"))?,
        )
    } else {
        None
    };
    let glob = if name == "find" {
        Some(
            call.pattern
                .as_deref()
                .ok_or("find requires a glob pattern")?,
        )
    } else {
        call.glob.as_deref()
    }
    .map(|p| {
        globset::Glob::new(p)
            .map(|g| g.compile_matcher())
            .map_err(|e| format!("Invalid glob: {e}"))
    })
    .transpose()?;
    let mut results = Vec::new();
    let mut seen = 0usize;
    let mut bytes = 0usize;
    let mut stopped = false;
    let mut unreadable = 0;
    if name == "ls" {
        let mut entries = fs::read_dir(&root)
            .map_err(|e| format!("Cannot list {}: {e}", root.display()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries.into_iter().skip(call.offset) {
            check()?;
            if results.len() >= limit || bytes >= MAX_OUTPUT {
                stopped = true;
                break;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            bytes += name.len() + 128;
            results.push(json!({"name":name,"kind":if kind.is_symlink(){"symlink"}else if kind.is_dir(){"directory"}else{"file"}}));
        }
    } else {
        let mut builder = ignore::WalkBuilder::new(&root);
        builder
            .hidden(!call.hidden)
            .follow_links(false)
            .sort_by_file_name(|a, b| a.cmp(b));
        'files: for entry in builder.build() {
            check()?;
            let entry = match entry {
                Ok(v) => v,
                Err(_) => {
                    unreadable += 1;
                    continue;
                }
            };
            if !entry.file_type().is_some_and(|v| v.is_file()) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .ok()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    entry
                        .path()
                        .file_name()
                        .map(Path::new)
                        .unwrap_or(entry.path())
                });
            if glob.as_ref().is_some_and(|g| !g.is_match(relative)) {
                continue;
            }
            if relative.to_str().is_none() {
                unreadable += 1;
                continue;
            }
            if name == "find" {
                if seen < call.offset {
                    seen += 1;
                    continue;
                }
                if results.len() >= limit || bytes >= MAX_OUTPUT {
                    stopped = true;
                    break;
                }
                bytes += relative.as_os_str().len() + 64;
                results.push(json!({"path":relative}));
                continue;
            }
            let contents = match load(entry.path()) {
                Ok(v) => v,
                Err(_) => {
                    unreadable += 1;
                    continue;
                }
            };
            if contents.contains(&0) {
                continue;
            }
            let text = match std::str::from_utf8(&contents) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for (number, line) in text.lines().enumerate() {
                check()?;
                if !matcher.as_ref().unwrap().is_match(line) {
                    continue;
                }
                if seen < call.offset {
                    seen += 1;
                    continue;
                }
                let (line, line_truncated) = clipped(line, 4096);
                let size = line.len() + relative.as_os_str().len() + 128;
                if results.len() >= limit || bytes + size > MAX_OUTPUT {
                    stopped = true;
                    break 'files;
                }
                bytes += size;
                results.push(json!({"path":relative,"line":number+1,"text":line,"lineTruncated":line_truncated}));
            }
        }
    }
    Ok(
        json!({"path":root,"results":results,"nextOffset":stopped.then_some(call.offset+results.len()),"truncated":stopped,"unreadableEntries":unreadable}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(root: &Path, name: &str, input: Value) -> Value {
        execute(name, &input.to_string(), root, &|| Ok(())).unwrap()
    }
    #[test]
    fn actual_files_can_be_created_read_edited_found_and_searched_with_unicode_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(
            root,
            "write",
            json!({"path":"中文 空格/example.txt","content":"one\ntwo\nthree\n"}),
        );
        let page = run(
            root,
            "read",
            json!({"path":"中文 空格/example.txt","offset":2,"limit":1}),
        );
        assert_eq!(page["content"], "two\n");
        assert_eq!(page["nextOffset"], 3);
        let changed = run(
            root,
            "edit",
            json!({"path":"中文 空格/example.txt","edits":[{"oldText":"one","newText":"FIRST"},{"oldText":"three","newText":"LAST"}]}),
        );
        assert_eq!(changed["committed"], true);
        assert!(changed["diff"].as_str().unwrap().contains("+LAST"));
        assert_eq!(
            run(root, "find", json!({"pattern":"**/*.txt"}))["results"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let found = run(root, "grep", json!({"pattern":"FIRST|LAST"}));
        assert_eq!(found["results"].as_array().unwrap().len(), 2);
        assert_eq!(found["results"][1]["line"], 3);
        assert_eq!(
            run(root, "ls", json!({}))["results"][0]["kind"],
            "directory"
        );
    }
    #[test]
    fn edits_are_atomic_unique_nonoverlapping_and_keep_bom_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x");
        let original = "\u{feff}alpha\r\nbeta\r\n";
        fs::write(&path, original).unwrap();
        assert!(execute("edit",&json!({"path":"x","edits":[{"oldText":"alpha","newText":"A"},{"oldText":"missing","newText":"X"}]}).to_string(),dir.path(),&||Ok(())).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        run(
            dir.path(),
            "edit",
            json!({"path":"x","edits":[{"oldText":"alpha\nbeta","newText":"new\nline"}]}),
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "\u{feff}new\r\nline\r\n"
        );
        fs::write(&path, "aaa").unwrap();
        assert!(execute(
            "edit",
            &json!({"path":"x","edits":[{"oldText":"aa","newText":"b"}]}).to_string(),
            dir.path(),
            &|| Ok(())
        )
        .is_err());
        fs::write(&path, "abcdef").unwrap();
        assert!(execute("edit",&json!({"path":"x","edits":[{"oldText":"abc","newText":"x"},{"oldText":"bc","newText":"y"}]}).to_string(),dir.path(),&||Ok(())).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "abcdef");
    }
    #[test]
    fn normal_account_access_is_not_confined_to_default_cwd_and_cancellation_prevents_writes() {
        let cwd = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = other.path().join("file.txt");
        run(
            cwd.path(),
            "write",
            json!({"path":path,"content":"outside cwd but explicitly addressed"}),
        );
        assert!(path.is_file());
        assert!(execute(
            "write",
            &json!({"path":"canceled","content":"no"}).to_string(),
            cwd.path(),
            &|| Err("canceled".into())
        )
        .is_err());
        assert!(!cwd.path().join("canceled").exists());
        assert!(execute("read", r#"{"path":"missing"}"#, cwd.path(), &|| Ok(())).is_err());
    }
}
