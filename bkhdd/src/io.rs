use std::io::{Read, Seek, SeekFrom};

pub struct BinInvertedReader<R>(R);

impl<R> BinInvertedReader<R>
where
    R: Read + Seek,
{
    pub fn new(reader: R) -> Self {
        Self(reader)
    }
}

impl<R: Read + Seek> Read for BinInvertedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        eprintln!("< READ: {:?}", self.0.stream_position().unwrap());
        let size = self.0.read(buf)?;
        eprintln!("> READ: {:?}", self.0.stream_position().unwrap());
        buf.iter_mut().for_each(|b| *b = !*b);
        Ok(size)
    }
}

impl<R: Seek> Seek for BinInvertedReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // Заглушка, я не знаю нахрена, возможно это и нужно
        // для проверки типа (а ты сука жив?), но это лишний syscall
        // так что лесом
        if let SeekFrom::Current(n) = pos {
            if n == 0 {
                return self.0.stream_position();
            }
        }
        eprintln!(
            "< SEEK: {:?} / pos: {:?}",
            self.0.stream_position().unwrap(),
            pos,
        );
        let res = self.0.seek(pos);
        eprintln!(
            "> SEEK: {:?} / Res: {:?}",
            self.0.stream_position().unwrap(),
            res
        );
        res
    }
}
