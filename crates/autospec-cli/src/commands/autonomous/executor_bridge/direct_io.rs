use std::fs::File;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

pub(super) fn normalize_git_argument(argument: &str) -> std::borrow::Cow<'_, str> {
    #[cfg(windows)]
    {
        if let Some(rest) = argument.strip_prefix(r"\\?\UNC\") {
            return std::borrow::Cow::Owned(format!(r"\\{rest}"));
        }
        if let Some(rest) = argument.strip_prefix(r"\\?\") {
            return std::borrow::Cow::Borrowed(rest);
        }
    }
    std::borrow::Cow::Borrowed(argument)
}

pub(super) fn git_with_path(
    repo: &Path,
    before: &[&str],
    path: &Path,
    after: &[&str],
) -> Result<(), String> {
    let path = path
        .to_str()
        .ok_or_else(|| "executor worktree path is not UTF-8".to_string())?;
    let path = normalize_git_argument(path);
    let args = before
        .iter()
        .copied()
        .chain(std::iter::once(path.as_ref()))
        .chain(after.iter().copied())
        .collect::<Vec<_>>();
    super::git(repo, &args)
}

pub(super) fn worktree_block_matches_path(block: &str, expected: &Path) -> bool {
    block
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .any(|observed| worktree_path_matches(observed, expected))
}

fn worktree_path_matches(observed: &str, expected: &Path) -> bool {
    if std::fs::canonicalize(observed).is_ok_and(|canonical| canonical == expected) {
        return true;
    }
    #[cfg(windows)]
    {
        let expected = expected.to_string_lossy();
        let expected = normalize_git_argument(expected.as_ref()).replace('/', "\\");
        return observed.replace('/', "\\").eq_ignore_ascii_case(&expected);
    }
    #[cfg(not(windows))]
    {
        Path::new(observed) == expected
    }
}

pub(super) fn read_exact_at_portable(
    file: &File,
    mut buffer: &mut [u8],
    mut offset: u64,
) -> std::io::Result<()> {
    while !buffer.is_empty() {
        #[cfg(unix)]
        let count = file.read_at(buffer, offset)?;
        #[cfg(windows)]
        let count = file.seek_read(buffer, offset)?;
        if count == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        offset += count as u64;
        buffer = &mut buffer[count..];
    }
    Ok(())
}

pub(super) fn read_at_portable(
    file: &File,
    buffer: &mut [u8],
    offset: u64,
) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        file.read_at(buffer, offset)
    }
    #[cfg(windows)]
    {
        file.seek_read(buffer, offset)
    }
}

pub(super) fn write_all_at_portable(
    file: &File,
    mut buffer: &[u8],
    mut offset: u64,
) -> std::io::Result<()> {
    while !buffer.is_empty() {
        #[cfg(unix)]
        let count = file.write_at(buffer, offset)?;
        #[cfg(windows)]
        let count = file.seek_write(buffer, offset)?;
        if count == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        offset += count as u64;
        buffer = &buffer[count..];
    }
    Ok(())
}
