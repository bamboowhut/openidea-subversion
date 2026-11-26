use crate::i18n::{tr, trf};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use url::Url;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Serialize)]
pub struct Info {
    pub path: String,
    pub url: String,
    pub repository: String,
    pub uuid: String,
    pub wc: String,
    pub revision: u64,
    pub kind: String,
    pub depth: String,
    pub copy_from: String,
    pub copy_rev: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub path: String,
    pub directory: bool,
    pub item: String,
    pub props: String,
    pub tree: bool,
    pub copied: bool,
    pub from: String,
    pub to: String,
    pub group: String,
    pub locked: bool,
}

impl Change {
    pub fn committable(&self) -> bool {
        !self.tree
            && self.props != "conflicted"
            && (matches!(
                self.item.as_str(),
                "added" | "modified" | "deleted" | "replaced"
            ) || self.item == "normal" && self.props == "modified")
    }
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: String,
    pub author: String,
    pub revision: String,
}

#[derive(Debug, Serialize)]
pub struct Revision {
    pub revision: u64,
    pub author: String,
    pub date: String,
    pub message: String,
    pub changes: Vec<RevisionPath>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RevisionPath {
    pub path: String,
    pub kind: String,
    pub action: String,
    pub from: String,
    pub from_rev: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Attribution {
    pub line: u32,
    pub revision: String,
    pub author: String,
    pub date: String,
}

#[derive(Debug, Serialize)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub binary: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct Conflict {
    pub text: bool,
    pub property: bool,
    pub tree: bool,
    pub operation: String,
    pub reason: String,
    pub action: String,
    pub base: String,
    pub mine: String,
    pub incoming: String,
}

fn parse(input: &str) -> Result<Document<'_>> {
    Document::parse(input).map_err(|e| trf!("Could not read SVN XML: {0}", e))
}

fn child<'a, 'i>(node: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn val(node: Node<'_, '_>, key: &str) -> String {
    child(node, key)
        .and_then(|n| n.text())
        .unwrap_or_default()
        .to_owned()
}

fn attr(node: Node<'_, '_>, key: &str) -> String {
    node.attribute(key).unwrap_or_default().to_owned()
}

pub fn info(input: &str) -> Result<Info> {
    infos(input)?
        .into_iter()
        .next()
        .ok_or_else(|| tr("SVN did not return target information").into())
}

pub fn infos(input: &str) -> Result<Vec<Info>> {
    let xml = parse(input)?;
    xml.root_element()
        .children()
        .filter(|n| n.has_tag_name("entry"))
        .map(info_node)
        .collect()
}

fn info_node(entry: Node<'_, '_>) -> Result<Info> {
    let repo = child(entry, "repository").ok_or(tr("SVN did not return repository information"))?;
    let wc = child(entry, "wc-info");
    Ok(Info {
        path: attr(entry, "path"),
        url: val(entry, "url"),
        repository: val(repo, "root"),
        uuid: val(repo, "uuid"),
        wc: wc.map(|wc| val(wc, "wcroot-abspath")).unwrap_or_default(),
        revision: entry
            .attribute("revision")
            .and_then(|r| r.parse().ok())
            .unwrap_or_default(),
        kind: attr(entry, "kind"),
        depth: wc.map(|wc| val(wc, "depth")).unwrap_or_default(),
        copy_from: wc.map(|wc| val(wc, "copy-from-url")).unwrap_or_default(),
        copy_rev: wc.and_then(|wc| val(wc, "copy-from-rev").parse().ok()),
    })
}

pub fn status(input: &str, cwd: &str) -> Result<Vec<Change>> {
    let xml = parse(input)?;
    let mut changes = Vec::new();
    for entry in xml.descendants().filter(|n| n.has_tag_name("entry")) {
        let Some(wc) = child(entry, "wc-status") else {
            continue;
        };
        let path = join(cwd, &attr(entry, "path"))?;
        let item = attr(wc, "item");
        let props = attr(wc, "props");
        let locked = child(wc, "lock").is_some();
        let tree = wc.attribute("tree-conflicted") == Some("true");
        if (item == "normal" || item == "none")
            && props != "modified"
            && props != "conflicted"
            && !locked
            && !tree
        {
            continue;
        }
        let moved = |key| -> Result<String> {
            wc.attribute(key)
                .map(|p| join(cwd, p))
                .transpose()
                .map(Option::unwrap_or_default)
        };
        changes.push(Change {
            path,
            directory: false,
            item,
            props,
            tree,
            locked,
            copied: wc.attribute("copied") == Some("true"),
            from: moved("moved-from")?,
            to: moved("moved-to")?,
            group: entry
                .parent()
                .filter(|n| n.has_tag_name("changelist"))
                .map(|n| attr(n, "name"))
                .unwrap_or_default(),
        });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

pub struct RemoteChange {
    pub path: String,
    pub revision: Option<u64>,
    pub item: String,
}

pub fn remote_status(input: &str, cwd: &str) -> Result<(Option<u64>, Vec<RemoteChange>)> {
    let xml = parse(input)?;
    let head = xml
        .descendants()
        .find(|node| node.has_tag_name("against"))
        .and_then(|node| node.attribute("revision"))
        .and_then(|revision| revision.parse().ok());
    let mut changes = Vec::new();
    for entry in xml.descendants().filter(|n| n.has_tag_name("entry")) {
        let Some(remote) = child(entry, "repos-status") else {
            continue;
        };
        let item = attr(remote, "item");
        if matches!(item.as_str(), "" | "none" | "normal") && attr(remote, "props") != "modified" {
            continue;
        }
        changes.push(RemoteChange {
            path: join(cwd, &attr(entry, "path"))?,
            revision: child(entry, "wc-status")
                .and_then(|n| n.attribute("revision"))
                .and_then(|r| r.parse().ok()),
            item,
        });
    }
    Ok((head, changes))
}

pub fn list(input: &str) -> Result<Vec<Entry>> {
    let xml = parse(input)?;
    Ok(xml
        .descendants()
        .filter(|n| n.has_tag_name("entry"))
        .map(|n| {
            let commit = child(n, "commit");
            Entry {
                name: val(n, "name"),
                kind: attr(n, "kind"),
                author: commit.map(|c| val(c, "author")).unwrap_or_default(),
                revision: commit.map(|c| attr(c, "revision")).unwrap_or_default(),
            }
        })
        .collect())
}

pub fn history(input: &str) -> Result<Vec<Revision>> {
    let xml = parse(input)?;
    Ok(xml
        .descendants()
        .filter(|n| n.has_tag_name("logentry"))
        .filter_map(|n| {
            Some(Revision {
                revision: n.attribute("revision")?.parse().ok()?,
                author: val(n, "author"),
                date: val(n, "date"),
                message: val(n, "msg"),
                changes: child(n, "paths")
                    .into_iter()
                    .flat_map(|p| p.children())
                    .filter(|p| p.has_tag_name("path"))
                    .map(|p| RevisionPath {
                        path: p.text().unwrap_or_default().into(),
                        kind: attr(p, "kind"),
                        action: attr(p, "action"),
                        from: attr(p, "copyfrom-path"),
                        from_rev: p.attribute("copyfrom-rev").and_then(|r| r.parse().ok()),
                    })
                    .collect(),
            })
        })
        .collect())
}

pub fn blame(input: &str) -> Result<Vec<Attribution>> {
    let xml = parse(input)?;
    Ok(xml
        .descendants()
        .filter(|n| n.has_tag_name("entry"))
        .filter_map(|n| {
            let c = child(n, "commit")?;
            Some(Attribution {
                line: n.attribute("line-number")?.parse().ok()?,
                revision: attr(c, "revision"),
                author: val(c, "author"),
                date: val(c, "date"),
            })
        })
        .collect())
}

pub fn properties(input: &str) -> Result<Vec<Property>> {
    let xml = parse(input)?;
    Ok(xml
        .descendants()
        .filter(|n| n.has_tag_name("property"))
        .map(|n| Property {
            name: attr(n, "name"),
            value: n.text().unwrap_or_default().into(),
            binary: n.attribute("encoding") == Some("base64"),
        })
        .collect())
}

pub fn conflict(input: &str) -> Result<Conflict> {
    let xml = parse(input)?;
    let mut result = Conflict::default();
    let mut details = Vec::new();
    for node in xml.descendants() {
        if node.has_tag_name("tree-conflict") {
            result.tree = true;
            details.push(node);
        }
        if node.has_tag_name("conflict") {
            match node.attribute("type") {
                Some("text") => {
                    result.text = true;
                    result.base = val(node, "prev-base-file");
                    result.mine = val(node, "prev-wc-file");
                    result.incoming = val(node, "cur-base-file");
                }
                Some("property") => result.property = true,
                _ => {}
            }
            details.push(node);
        }
    }
    if let Some(node) = details.first() {
        result.operation = attr(*node, "operation");
        result.reason = attr(*node, "reason");
        result.action = attr(*node, "action");
    }
    Ok(result)
}

pub fn absolute(p: &str) -> bool {
    p.starts_with('/')
        || p.starts_with('\\')
        || p.as_bytes()
            .get(1..3)
            .is_some_and(|s| s == b":/" || s == b":\\")
}

pub fn canonical(p: &str) -> Result<String> {
    let p = if p.as_bytes().get(1) == Some(&b':') || p.starts_with('\\') {
        p.replace('\\', "/")
    } else {
        p.to_owned()
    };
    if !absolute(&p) || p.contains(['\0', '\r', '\n']) {
        return Err(tr("A valid absolute path is required").into());
    }
    if p.starts_with("//?/") || p.starts_with("//./") {
        return Err(tr("Device paths are unsupported").into());
    }
    let (prefix, tail) = if let Some(rest) = p.strip_prefix("//") {
        ("//", rest)
    } else if let Some(rest) = p.strip_prefix('/') {
        ("/", rest)
    } else {
        (&p[..3], &p[3..])
    };
    let mut parts = Vec::new();
    for part in tail.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or(tr("The path goes above the root directory"))?;
            }
            x => parts.push(x),
        }
    }
    if prefix == "//" && parts.len() < 2 {
        return Err(tr("A UNC path needs a server and share name").into());
    }
    Ok(format!("{prefix}{}", parts.join("/")))
}

pub fn join(root: &str, path: &str) -> Result<String> {
    if absolute(path) {
        return canonical(path);
    }
    let path = if windows(root) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    canonical(&format!("{}/{path}", root.trim_end_matches('/')))
}

pub fn windows(path: &str) -> bool {
    path.starts_with("//") || path.as_bytes().get(1) == Some(&b':')
}

pub fn status_target(input: &str) -> Result<String> {
    let xml = parse(input)?;
    child(xml.root_element(), "target")
        .map(|n| attr(n, "path"))
        .ok_or_else(|| tr("SVN did not report the requested status target").into())
}

pub fn local_argument(root: &str, path: &str) -> String {
    if !absolute(path) {
        return path.into();
    }
    if key(path) == key(root) {
        return ".".into();
    }
    let root = root.trim_end_matches('/');
    if let Some(head) = path.get(..root.len())
        && key(head) == key(root)
    {
        if let Some(tail) = path[root.len()..].strip_prefix('/') {
            return format!("./{tail}");
        }
        if let Some(peg) = path[root.len()..].strip_prefix('@') {
            return format!(".@{peg}");
        }
    }
    path.into()
}

pub fn key(path: &str) -> String {
    if windows(path) {
        path.to_lowercase()
    } else {
        path.into()
    }
}

pub fn within(root: &str, path: &str) -> bool {
    let root = key(root);
    let path = key(path);
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

pub fn project_path(root: &str, input: &str) -> Result<String> {
    let path = join(root, input)?;
    let root_len = root.trim_end_matches('/').split('/').count();
    let meta = path.split('/').skip(root_len).any(|p| {
        [".svn", "_svn", ".git", ".openidea"]
            .iter()
            .any(|m| p.trim_end_matches([' ', '.']).eq_ignore_ascii_case(m))
            || windows(root) && (p.contains(':') || p.ends_with([' ', '.']))
    });
    if !within(root, &path) || meta {
        return Err(
            tr("The target is outside the project or inside an administrative directory").into(),
        );
    }
    Ok(path)
}

pub fn parent(path: &str) -> Option<String> {
    let (dir, name) = path.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    if dir.is_empty() || dir.ends_with(':') {
        Some(format!("{dir}/"))
    } else {
        Some(dir.into())
    }
}

pub fn leaf(name: &str) -> Result<&str> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0', '\r', '\n', ':'])
        || name.ends_with(['.', ' '])
    {
        return Err(tr("Enter a single valid name").into());
    }
    Ok(name)
}

pub fn url(input: &str) -> Result<String> {
    let parsed = Url::parse(input.trim()).map_err(|_| tr("Enter a complete SVN repository URL"))?;
    if !matches!(
        parsed.scheme(),
        "file" | "http" | "https" | "svn" | "svn+ssh"
    ) || parsed.password().is_some()
        || (!parsed.username().is_empty() && parsed.scheme() != "svn+ssh")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || (!matches!(parsed.scheme(), "file") && parsed.host().is_none())
        || (parsed.scheme() == "file" && !parsed.path().starts_with('/'))
    {
        return Err(tr("Enter a valid SVN URL without a password").into());
    }
    Ok(parsed.into())
}

pub fn url_path(base: &str, path: &str) -> Result<String> {
    let mut base = Url::parse(base).map_err(|_| tr("Invalid SVN URL"))?;
    let mut segments = base
        .path_segments_mut()
        .map_err(|_| tr("Invalid SVN URL"))?;
    segments.pop_if_empty();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        if part == "." || part == ".." {
            return Err(tr("Invalid repository path").into());
        }
        segments.push(part);
    }
    drop(segments);
    Ok(base.into())
}

pub fn peg(path: &str) -> String {
    if path.contains('@') {
        format!("{path}@")
    } else {
        path.into()
    }
}

pub fn pegged(path: &str, rev: u64) -> String {
    format!("{path}@{rev}")
}

pub fn revision(value: &str) -> Result<String> {
    let s = value.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("head") {
        return Ok("HEAD".into());
    }
    s.parse::<u64>()
        .map(|r| r.to_string())
        .map_err(|_| tr("Revision must be a non-negative integer or HEAD").into())
}

pub fn revisions(value: &str) -> Result<String> {
    let numbers = value
        .split(',')
        .map(|s| {
            let r = s.trim().parse::<i64>().map_err(|_| {
                tr("Enter comma-separated revisions. Negative revisions reverse a merge.")
            })?;
            if r == 0 || r == i64::MIN {
                return Err(tr("Revision must be non-zero and within range"));
            }
            Ok(r.to_string())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(numbers.join(","))
}

pub fn decode(bytes: &[u8], encoding: &str) -> Result<String> {
    if let Some((codec, length)) = encoding_rs::Encoding::for_bom(bytes) {
        return codec
            .decode_without_bom_handling_and_without_replacement(&bytes[length..])
            .map(|s| s.into_owned())
            .ok_or_else(|| tr("The file encoding does not match its byte order mark").into());
    }
    if bytes.contains(&0) {
        return Err(tr("Binary files cannot be shown in a text diff").into());
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.into());
    }
    let codec = encoding_rs::Encoding::for_label(encoding.as_bytes())
        .ok_or(tr("Select the file's character encoding"))?;
    codec
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|s| s.into_owned())
        .ok_or_else(|| tr("Cannot decode the file with the selected encoding").into())
}
