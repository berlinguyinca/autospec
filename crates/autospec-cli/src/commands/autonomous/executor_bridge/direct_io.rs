use std::fs::File;

#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;

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
