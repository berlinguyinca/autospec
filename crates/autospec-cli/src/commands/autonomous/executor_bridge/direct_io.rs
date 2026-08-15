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

pub(super) fn matching_worktree_blocks<'a>(
    registry: &'a str,
    expected: &Path,
) -> Result<Vec<&'a str>, String> {
    registry
        .split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .filter_map(|block| {
            block
                .lines()
                .find_map(|line| line.strip_prefix("worktree "))
                .map(|observed| {
                    worktree_path_matches(observed, expected).map(|matched| (block, matched))
                })
        })
        .filter_map(|result| match result {
            Ok((block, true)) => Some(Ok(block)),
            Ok((_, false)) => None,
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn worktree_path_matches(observed: &str, expected: &Path) -> Result<bool, String> {
    match std::fs::canonicalize(observed) {
        Ok(canonical) => Ok(canonical == expected),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(windows)]
            {
                let expected = expected.to_string_lossy();
                let expected = normalize_git_argument(expected.as_ref()).replace('/', "\\");
                Ok(observed.replace('/', "\\").eq_ignore_ascii_case(&expected))
            }
            #[cfg(not(windows))]
            {
                Ok(Path::new(observed) == expected)
            }
        }
        Err(error) => Err(format!(
            "canonicalize registered worktree {observed}: {error}"
        )),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_worktree_match_never_falls_back_after_identity_mismatch() {
        let root = std::env::temp_dir().join(format!(
            "autospec-worktree-path-match-{}",
            std::process::id()
        ));
        let observed = root.join("observed");
        let expected = root.join("expected");
        std::fs::create_dir_all(&observed).expect("create observed worktree");
        std::fs::create_dir_all(&expected).expect("create expected worktree");
        let expected = std::fs::canonicalize(expected).expect("canonical expected worktree");
        assert!(!worktree_path_matches(
            observed.to_str().expect("UTF-8 observed worktree"),
            &expected,
        )
        .expect("compare live worktree identities"));
        std::fs::remove_dir_all(root).expect("remove worktree identity fixture");
    }

    #[test]
    #[cfg(windows)]
    fn missing_worktree_matches_conventional_and_verbatim_spelling() {
        let suffix = format!("autospec-missing-worktree-{}", std::process::id());
        let conventional = format!(r"C:\{suffix}");
        let verbatim = format!(r"\\?\C:\{suffix}");
        assert!(worktree_path_matches(&conventional, Path::new(&verbatim))
            .expect("compare missing Windows worktree spellings"));
    }
}
