use crate::error::{Result, ShoreError};
use crate::model::FileStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawFile {
    pub status: FileStatus,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub similarity: Option<u16>,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
}

impl RawFile {
    pub fn key(&self) -> String {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("raw file has at least one path")
            .clone()
    }

    pub fn is_submodule(&self) -> bool {
        self.old_mode.as_deref() == Some("160000") || self.new_mode.as_deref() == Some("160000")
    }

    pub fn is_mode_only(&self) -> bool {
        self.status == FileStatus::Modified
            && self.old_mode != self.new_mode
            && !self.is_submodule()
    }
}

pub(crate) fn parse_raw(raw: &[u8]) -> Result<Vec<RawFile>> {
    let fields = raw
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| {
            std::str::from_utf8(field)
                .map(str::to_owned)
                .map_err(|error| {
                    ShoreError::Message(format!("git raw output is not utf-8: {error}"))
                })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let header = &fields[index];
        index += 1;
        let parsed_header = parse_header(header)?;

        let path = fields
            .get(index)
            .ok_or_else(|| ShoreError::Message(format!("missing path for raw header {header}")))?
            .clone();
        index += 1;
        let second_path = if matches!(
            parsed_header.status,
            FileStatus::Renamed | FileStatus::Copied
        ) {
            let second_path = fields
                .get(index)
                .ok_or_else(|| {
                    ShoreError::Message(format!("missing second path for raw header {header}"))
                })?
                .clone();
            index += 1;
            Some(second_path)
        } else {
            None
        };

        let (old_path, new_path) = match parsed_header.status {
            FileStatus::Modified => (Some(path.clone()), Some(path)),
            FileStatus::Added => (None, Some(path)),
            FileStatus::Deleted => (Some(path), None),
            FileStatus::Renamed | FileStatus::Copied => (Some(path), second_path),
        };
        files.push(RawFile {
            status: parsed_header.status,
            old_mode: mode(parsed_header.old_mode),
            new_mode: mode(parsed_header.new_mode),
            old_oid: oid(parsed_header.old_oid),
            new_oid: oid(parsed_header.new_oid),
            similarity: parsed_header.similarity,
            old_path,
            new_path,
        });
    }

    Ok(files)
}

#[derive(Debug)]
struct RawHeader<'a> {
    old_mode: &'a str,
    new_mode: &'a str,
    old_oid: &'a str,
    new_oid: &'a str,
    status: FileStatus,
    similarity: Option<u16>,
}

fn parse_header(header: &str) -> Result<RawHeader<'_>> {
    let parts = header.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err(ShoreError::Message(format!(
            "expected 5 raw header fields, got {} in {header}",
            parts.len()
        )));
    }
    let status = parts[4];
    let file_status = match status.chars().next() {
        Some('M') => FileStatus::Modified,
        Some('A') => FileStatus::Added,
        Some('D') => FileStatus::Deleted,
        Some('R') => FileStatus::Renamed,
        Some('C') => FileStatus::Copied,
        _ => {
            return Err(ShoreError::Message(format!(
                "unsupported raw status {status}"
            )));
        }
    };
    let similarity = status
        .get(1..)
        .filter(|score| !score.is_empty())
        .map(|score| {
            score.parse().map_err(|error| {
                ShoreError::Message(format!("invalid similarity {score}: {error}"))
            })
        })
        .transpose()?;

    Ok(RawHeader {
        old_mode: parts[0].trim_start_matches(':'),
        new_mode: parts[1],
        old_oid: parts[2],
        new_oid: parts[3],
        status: file_status,
        similarity,
    })
}

fn mode(mode: &str) -> Option<String> {
    (mode != "000000").then(|| mode.to_owned())
}

fn oid(oid: &str) -> Option<String> {
    (!oid.chars().all(|ch| ch == '0')).then(|| oid.to_owned())
}
