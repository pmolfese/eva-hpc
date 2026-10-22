use crate::{error::io, Error, Limits, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub data: u64,
    pub start: u64,
    pub samples: u64,
    pub offsets: Vec<u64>,
}
#[derive(Debug)]
pub(crate) struct Index {
    pub blocks: Vec<Block>,
    pub channels: usize,
    pub rate: u32,
    pub samples: u64,
    pub file_len: u64,
}
fn word(f: &mut File, p: &Path) -> Result<u32> {
    let mut b = [0; 4];
    f.read_exact(&mut b).map_err(|e| io(p, e))?;
    Ok(u32::from_le_bytes(b))
}
pub(crate) fn index(path: &Path, limits: &Limits) -> Result<Index> {
    let mut f = File::open(path).map_err(|e| io(path, e))?;
    let len = f.metadata().map_err(|e| io(path, e))?.len();
    let mut pos = 0u64;
    let mut blocks = Vec::new();
    let mut previous: Option<(u64, Vec<u64>, u64)> = None;
    let mut config = None;
    let mut total = 0u64;
    let mut index_bytes = 0usize;
    while pos < len {
        if blocks.len() >= limits.max_blocks {
            return Err(Error::Limit("block count".into()));
        }
        let flag = word(&mut f, path)?;
        let (size, offsets, samples) = match flag {
            1 => {
                let header = u64::from(word(&mut f, path)?);
                let size = u64::from(word(&mut f, path)?);
                let n = word(&mut f, path)? as usize;
                if n == 0 || n > limits.max_channels {
                    return Err(Error::Limit("channel count".into()));
                }
                let min_header = 16 + n as u64 * 8;
                if header < min_header
                    || header > len - pos
                    || size == 0
                    || size > len - pos - header
                {
                    return Err(Error::Invalid(format!(
                        "{}: invalid header/data size at {pos}",
                        path.display()
                    )));
                }
                let mut offsets = Vec::with_capacity(n);
                for _ in 0..n {
                    offsets.push(u64::from(word(&mut f, path)?));
                }
                let mut rate = 0;
                for ch in 0..n {
                    let v = word(&mut f, path)?;
                    if v & 255 != 32 {
                        return Err(Error::Unsupported(format!("{}-bit samples", v & 255)));
                    }
                    let r = v >> 8;
                    if r == 0 || (ch > 0 && r != rate) {
                        return Err(Error::Unsupported("mixed/zero channel rates".into()));
                    }
                    rate = r;
                }
                if config.is_some_and(|c| c != (n, rate)) {
                    return Err(Error::Unsupported(
                        "changing channel count/sample rate".into(),
                    ));
                }
                config = Some((n, rate));
                if offsets[0] != 0 {
                    return Err(Error::Unsupported("nonzero first channel offset".into()));
                }
                let mut samples = None;
                for ch in 0..n {
                    let end = offsets.get(ch + 1).copied().unwrap_or(size);
                    let width = end
                        .checked_sub(offsets[ch])
                        .filter(|w| *w > 0 && *w % 4 == 0)
                        .ok_or_else(|| Error::Invalid("invalid channel offsets".into()))?;
                    if end > size {
                        return Err(Error::Invalid("channel offset exceeds block".into()));
                    }
                    if samples.is_some_and(|s| s != width / 4) {
                        return Err(Error::Unsupported("unequal channel sample counts".into()));
                    }
                    samples = Some(width / 4);
                }
                f.seek(SeekFrom::Start(pos + header))
                    .map_err(|e| io(path, e))?;
                let h = (size, offsets, samples.unwrap());
                previous = Some(h.clone());
                h
            }
            0 => previous
                .clone()
                .ok_or_else(|| Error::Invalid("reused header before first header".into()))?,
            _ => return Err(Error::Invalid(format!("unknown block flag {flag}"))),
        };
        let data = f.stream_position().map_err(|e| io(path, e))?;
        if size > len - data {
            return Err(Error::Invalid("truncated data block".into()));
        }
        index_bytes = index_bytes
            .checked_add(std::mem::size_of::<Block>() + offsets.len() * 8)
            .ok_or_else(|| Error::Limit("index size overflow".into()))?;
        if index_bytes > limits.max_index_bytes {
            return Err(Error::Limit("block index memory".into()));
        }
        blocks.push(Block {
            data,
            start: total,
            samples,
            offsets,
        });
        total = total
            .checked_add(samples)
            .ok_or_else(|| Error::Limit("sample count overflow".into()))?;
        pos = data + size;
        f.seek(SeekFrom::Start(pos)).map_err(|e| io(path, e))?;
    }
    let (channels, rate) = config.ok_or_else(|| Error::Invalid("empty signal".into()))?;
    Ok(Index {
        blocks,
        channels,
        rate,
        samples: total,
        file_len: len,
    })
}
