use enum_iterator::IntoEnumIterator;
use serde::{Deserialize, Serialize};
use std::io::{self, BufReader, Read, Write};

#[derive(Debug, Serialize, Deserialize, IntoEnumIterator)]
pub enum CompressionMethod {
    NoCompression,
    Bzip2,
    Gzip,
    Zstd,
}

pub struct CompressedData {
    pub method: CompressionMethod,
    pub data: Vec<u8>,
}

fn decompress_reader<'a, R: Read + 'a>(
    method: CompressionMethod,
    stream: R,
) -> Result<Box<dyn Read + 'a>, io::Error> {
    let buf_reader = BufReader::new(stream);
    let decompress_reader: Box<dyn Read> = match method {
        CompressionMethod::Bzip2 => Box::new(bzip2::bufread::BzDecoder::new(buf_reader)),
        CompressionMethod::Gzip => Box::new(flate2::read::GzDecoder::new(buf_reader)),
        CompressionMethod::Zstd => Box::new(zstd::stream::read::Decoder::new(buf_reader)?),
        CompressionMethod::NoCompression => Box::new(buf_reader),
    };
    Ok(decompress_reader)
}

pub fn decompress(compressed_data: CompressedData) -> Result<Vec<u8>, io::Error> {
    let mut reader = decompress_reader(compressed_data.method, &compressed_data.data[..])?;
    let mut uncompressed_data = vec![];
    reader.read_to_end(&mut uncompressed_data)?;
    Ok(uncompressed_data)
}

pub fn compress(method: CompressionMethod, data: &[u8]) -> Result<CompressedData, io::Error> {
    let data = match method {
        CompressionMethod::Bzip2 => {
            let mut e = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::Best);
            e.write_all(data)?;
            e.finish()?
        }
        CompressionMethod::Gzip => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data)?;
            e.finish()?
        }
        CompressionMethod::Zstd => {
            let mut e = zstd::stream::write::Encoder::new(Vec::new(), 0).unwrap();
            e.write_all(data)?;
            e.finish()?
        }
        CompressionMethod::NoCompression => data.to_vec(),
    };
    Ok(CompressedData { method, data })
}

pub fn compress_best(data: &[u8]) -> Result<CompressedData, io::Error> {
    let mut candidates = vec![];
    for method in CompressionMethod::into_enum_iter() {
        candidates.push(compress(method, data)?);
    }

    Ok(candidates
        .into_iter()
        .min_by(|a, b| a.data.len().cmp(&b.data.len()))
        .unwrap())
}
