use std::io::{Read, Seek, SeekFrom};

pub struct BinInvertedReader<R>(R);

impl<R> BinInvertedReader<R>
where
    R: Read + Seek,
{
    pub fn new(reader: R) -> Self {
        Self(reader)
    }

    pub fn into_inner(self) -> R {
        self.0
    }
}

impl<R: Read + Seek> Read for BinInvertedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let size = self.0.read(buf)?;
        buf.iter_mut().for_each(|b| *b = !*b);
        Ok(size)
    }
}

impl<R: Seek> Seek for BinInvertedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

pub struct ReverseReader<R>(R);

impl<R> ReverseReader<R>
where
    R: Read + Seek,
{
    pub fn new(reader: R) -> Self {
        Self(reader)
    }

    pub fn into_inner(self) -> R {
        self.0
    }
}

impl<R: Read + Seek> Read for ReverseReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = buf.len();
        let _pos = self.0.seek(SeekFrom::Current(-(len as i64)))?;
        let size = self.0.read(buf)?;
        let _pos = self.0.seek(SeekFrom::Current(-(len as i64)))?;
        Ok(size)
    }
}

impl<R: Seek> Seek for ReverseReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}
