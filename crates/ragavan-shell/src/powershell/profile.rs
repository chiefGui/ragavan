use super::Error;
use std::{
    ffi::OsString,
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const START_MARKER: &str = "# >>> ragavan >>>";
const END_MARKER: &str = "# <<< ragavan <<<";

static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Profile {
    path: PathBuf,
    original: Option<Vec<u8>>,
    text: String,
    encoding: Encoding,
}

impl Profile {
    pub(super) fn read(path: &Path) -> Result<Option<Self>, Error> {
        let storage_path = storage_path(path)?;
        let bytes = match fs::read(&storage_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::ReadProfile {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let (text, encoding) = Encoding::decode(&bytes).map_err(|source| Error::DecodeProfile {
            path: path.to_owned(),
            source,
        })?;

        Ok(Some(Self {
            path: storage_path,
            original: Some(bytes),
            text,
            encoding,
        }))
    }

    pub(super) fn empty(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            original: None,
            text: String::new(),
            encoding: Encoding::Utf8,
        }
    }

    pub(super) fn install(&mut self) -> Result<bool, Error> {
        let integration = integration_range(&self.text).map_err(|()| Error::MalformedProfile {
            path: self.path.clone(),
        })?;
        let newline = preferred_newline(&self.text);
        let block = integration_block(newline);
        let range = integration.unwrap_or(self.text.len()..self.text.len());
        let prefix = &self.text[..range.start];
        let suffix = &self.text[range.end..];
        let mut updated =
            String::with_capacity(prefix.len() + suffix.len() + block.len() + newline.len());
        updated.push_str(prefix);
        if !prefix.is_empty() {
            updated.push_str(newline);
        }
        updated.push_str(&block);
        updated.push_str(suffix);

        if updated == self.text {
            return Ok(false);
        }

        self.write(updated)?;
        Ok(true)
    }

    pub(super) fn uninstall(&mut self) -> Result<bool, Error> {
        let Some(range) = integration_range(&self.text).map_err(|()| Error::MalformedProfile {
            path: self.path.clone(),
        })?
        else {
            return Ok(false);
        };
        let prefix = &self.text[..range.start];
        let suffix = &self.text[range.end..];
        let mut updated = String::with_capacity(prefix.len() + suffix.len());
        updated.push_str(prefix);
        if !prefix.is_empty()
            && !suffix.is_empty()
            && !prefix.ends_with(['\r', '\n'])
            && !suffix.starts_with(['\r', '\n'])
        {
            updated.push_str(preferred_newline(&self.text));
        }
        updated.push_str(suffix);

        self.write(updated)?;
        Ok(true)
    }

    fn write(&mut self, text: String) -> Result<(), Error> {
        let bytes = self.encoding.encode(&text);
        write_atomically(&self.path, self.original.as_deref(), &bytes)?;
        self.original = Some(bytes);
        self.text = text;
        Ok(())
    }
}

fn storage_path(path: &Path) -> Result<PathBuf, Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(path).map_err(|source| Error::ReadProfile {
                path: path.to_owned(),
                source,
            })
        }
        Ok(_) => Ok(path.to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_owned()),
        Err(source) => Err(Error::ReadProfile {
            path: path.to_owned(),
            source,
        }),
    }
}

fn integration_range(text: &str) -> Result<Option<std::ops::Range<usize>>, ()> {
    let mut start = None;
    let mut end = None;
    let mut offset = 0;

    for line in text.split_inclusive('\n') {
        let line_end = offset + line.len();
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);

        if content == START_MARKER {
            if start.replace(offset).is_some() {
                return Err(());
            }
        } else if content == END_MARKER && end.replace(line_end).is_some() {
            return Err(());
        }

        offset = line_end;
    }

    match (start, end) {
        (None, None) => Ok(None),
        (Some(start), Some(end)) if start < end => {
            let start = preceding_newline(text, start).unwrap_or(start);
            Ok(Some(start..end))
        }
        _ => Err(()),
    }
}

fn preceding_newline(text: &str, offset: usize) -> Option<usize> {
    let prefix = text.as_bytes().get(..offset)?;
    if prefix.ends_with(b"\r\n") {
        Some(offset - 2)
    } else if prefix.ends_with(b"\n") {
        Some(offset - 1)
    } else {
        None
    }
}

fn preferred_newline(text: &str) -> &'static str {
    text.find('\n').map_or_else(
        || if cfg!(windows) { "\r\n" } else { "\n" },
        |newline| {
            if newline > 0 && text.as_bytes()[newline - 1] == b'\r' {
                "\r\n"
            } else {
                "\n"
            }
        },
    )
}

fn integration_block(newline: &str) -> String {
    [
        START_MARKER,
        "# Managed by Ragavan. Run `ragavan uninstall` to remove.",
        "$__RagavanBootstrapCommand = Get-Command ragavan -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1",
        "if ($null -ne $__RagavanBootstrapCommand) {",
        "    Invoke-Expression (& $__RagavanBootstrapCommand hook powershell | Out-String)",
        "}",
        "Remove-Variable __RagavanBootstrapCommand -ErrorAction SilentlyContinue",
        END_MARKER,
        "",
    ]
    .join(newline)
}

fn write_atomically(path: &Path, original: Option<&[u8]>, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| Error::InvalidProfilePath {
        path: path.to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|source| Error::WriteProfile {
        path: path.to_owned(),
        source,
    })?;

    let (temporary, mut file) = TemporaryFile::create(path)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| Error::WriteProfile {
            path: path.to_owned(),
            source,
        })?;
    drop(file);

    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::ReadProfile {
                path: path.to_owned(),
                source,
            });
        }
    };
    if current.as_deref() != original {
        return Err(Error::ProfileChanged {
            path: path.to_owned(),
        });
    }

    if current.is_some() {
        let permissions = fs::metadata(path)
            .map_err(|source| Error::ReadProfile {
                path: path.to_owned(),
                source,
            })?
            .permissions();
        fs::set_permissions(temporary.path(), permissions).map_err(|source| {
            Error::WriteProfile {
                path: path.to_owned(),
                source,
            }
        })?;
    }

    fs::rename(temporary.path(), path).map_err(|source| Error::WriteProfile {
        path: path.to_owned(),
        source,
    })?;
    Ok(())
}

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn create(profile: &Path) -> Result<(Self, File), Error> {
        let parent = profile.parent().ok_or_else(|| Error::InvalidProfilePath {
            path: profile.to_owned(),
        })?;
        let name = profile
            .file_name()
            .ok_or_else(|| Error::InvalidProfilePath {
                path: profile.to_owned(),
            })?;

        for _ in 0..100 {
            let sequence = NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = OsString::from(".");
            temporary_name.push(name);
            temporary_name.push(format!(".ragavan-{}-{sequence}.tmp", std::process::id()));
            let path = parent.join(temporary_name);

            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self(path), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(Error::WriteProfile {
                        path: profile.to_owned(),
                        source,
                    });
                }
            }
        }

        Err(Error::WriteProfile {
            path: profile.to_owned(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a temporary profile file",
            ),
        })
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        match fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16LittleEndian,
    Utf16BigEndian,
}

impl Encoding {
    fn decode(bytes: &[u8]) -> Result<(String, Self), DecodeError> {
        if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
            return std::str::from_utf8(bytes)
                .map(|text| (text.to_owned(), Self::Utf8Bom))
                .map_err(DecodeError::Utf8);
        }
        if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
            return decode_utf16(bytes, u16::from_le_bytes)
                .map(|text| (text, Self::Utf16LittleEndian));
        }
        if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
            return decode_utf16(bytes, u16::from_be_bytes)
                .map(|text| (text, Self::Utf16BigEndian));
        }

        std::str::from_utf8(bytes)
            .map(|text| (text.to_owned(), Self::Utf8))
            .map_err(DecodeError::Utf8)
    }

    fn encode(self, text: &str) -> Vec<u8> {
        match self {
            Self::Utf8 => text.as_bytes().to_vec(),
            Self::Utf8Bom => {
                let mut bytes = Vec::with_capacity(3 + text.len());
                bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
                bytes.extend_from_slice(text.as_bytes());
                bytes
            }
            Self::Utf16LittleEndian => encode_utf16(text, [0xff, 0xfe], u16::to_le_bytes),
            Self::Utf16BigEndian => encode_utf16(text, [0xfe, 0xff], u16::to_be_bytes),
        }
    }
}

fn decode_utf16(bytes: &[u8], decode: fn([u8; 2]) -> u16) -> Result<String, DecodeError> {
    let chunks = bytes.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(DecodeError::OddUtf16ByteCount);
    }
    let code_units: Vec<_> = chunks.map(|chunk| decode([chunk[0], chunk[1]])).collect();
    String::from_utf16(&code_units).map_err(DecodeError::Utf16)
}

fn encode_utf16(text: &str, bom: [u8; 2], encode: fn(u16) -> [u8; 2]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + text.len() * 2);
    bytes.extend_from_slice(&bom);
    for code_unit in text.encode_utf16() {
        bytes.extend_from_slice(&encode(code_unit));
    }
    bytes
}

#[derive(Debug)]
pub(crate) enum DecodeError {
    Utf8(std::str::Utf8Error),
    Utf16(std::string::FromUtf16Error),
    OddUtf16ByteCount,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(source) => source.fmt(formatter),
            Self::Utf16(source) => source.fmt(formatter),
            Self::OddUtf16ByteCount => {
                formatter.write_str("UTF-16 content has an incomplete code unit")
            }
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Utf8(source) => Some(source),
            Self::Utf16(source) => Some(source),
            Self::OddUtf16ByteCount => None,
        }
    }
}
