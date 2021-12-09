use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
};

pub enum Reader {
    File(File),
    Inverted(BinInvertedReader<File>),
}

impl Reader {
    pub fn new(reader: File) -> Self {
        Self::File(reader)
    }

    pub fn inverted(reader: File) -> Self {
        let bir = BinInvertedReader::new(reader);
        Self::Inverted(bir)
    }

    pub fn into_inner(self) -> File {
        match self {
            Self::File(h) => h,
            Self::Inverted(h) => h.into_inner(),
        }
    }

    pub fn metadata(&self) -> std::io::Result<fs::Metadata> {
        match self {
            Self::File(h) => h.metadata(),
            Self::Inverted(h) => h.as_ref().metadata(),
        }
    }
}

impl AsRef<File> for Reader {
    fn as_ref(&self) -> &File {
        match self {
            Self::File(h) => h,
            Self::Inverted(h) => h.as_ref(),
        }
    }
}

impl AsMut<File> for Reader {
    fn as_mut(&mut self) -> &mut File {
        match self {
            Self::File(h) => h,
            Self::Inverted(h) => h.as_mut(),
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(h) => h.read(buf),
            Self::Inverted(h) => h.read(buf),
        }
    }
}

impl Seek for Reader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(h) => h.seek(pos),
            Self::Inverted(h) => h.seek(pos),
        }
    }
}

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

impl<R> AsRef<R> for BinInvertedReader<R> {
    fn as_ref(&self) -> &R {
        &self.0
    }
}

impl<R> AsMut<R> for BinInvertedReader<R> {
    fn as_mut(&mut self) -> &mut R {
        &mut self.0
    }
}

impl<R: Read> Read for BinInvertedReader<R> {
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
