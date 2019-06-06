use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::{self, SerializeTuple, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

/*
/// Same as u16, but serialized with 1 to 3 bytes. If the value is above
/// 0x7f, the top bit is set and the remaining value is stored in the next
/// bytes. Each byte follows the same pattern until the 3rd byte. The 3rd
/// byte, if needed, uses all 8 bits to store the last byte of the original
/// value.
pub struct ShortUsize(pub u16);

impl Serialize for ShortUsize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Pass a non-zero value to serialize_tuple() so that serde_json will
        // generate an open bracket.
        let mut seq = serializer.serialize_tuple(1)?;

        let mut rem_len = self.0;
        loop {
            let mut elem = (rem_len & 0x7f) as u8;
            rem_len >>= 7;
            if rem_len == 0 {
                seq.serialize_element(&elem)?;
                break;
            } else {
                elem |= 0x80;
                seq.serialize_element(&elem)?;
            }
        }
        seq.end()
    }
}

struct ShortLenVisitor;

impl<'de> Visitor<'de> for ShortLenVisitor {
    type Value = ShortUsize;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a multi-byte length")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<ShortUsize, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut len: usize = 0;
        let mut size = 0;
        loop {
            let elem: u8 = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(size, &self))?;

            dbg!(size);
            dbg!(size * 7);
            len |= (elem as usize & 0x7f) << (size * 7);
            dbg!("here");
            size += 1;
            dbg!("here2");

            if elem as u16 & 0x80 == 0 {
                break;
            }

            if size > size_of::<u16>() + 1 {
                dbg!("here4");
                dbg!(size);
                return Err(de::Error::invalid_length(size, &self));
            }
        }
        dbg!("here3");
        Ok(ShortUsize(len as u16))
    }
}

impl<'de> Deserialize<'de> for ShortUsize {
    fn deserialize<D>(deserializer: D) -> Result<ShortUsize, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_tuple(9, ShortLenVisitor)
    }
}
*/

/// If you don't want to use the ShortVec newtype, you can do ShortVec
/// serialization on an ordinary vector with the following field annotation:
///
/// #[serde(with = "short_vec")]
///
pub fn serialize<S: Serializer, T: Serialize>(
    elements: &[T],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    // Pass a non-zero value to serialize_tuple() so that serde_json will
    // generate an open bracket.
    let mut seq = serializer.serialize_tuple(1)?;

    let len = elements.len();
    if len > std::u8::MAX as usize {
        return Err(ser::Error::custom("length too large"));
    }
    let short_len = len as u8;
    seq.serialize_element(&short_len)?;

    for element in elements {
        seq.serialize_element(element)?;
    }
    seq.end()
}

struct ShortVecVisitor<T> {
    _t: PhantomData<T>,
}

impl<'de, T> Visitor<'de> for ShortVecVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a Vec with a multi-byte length")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Vec<T>, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let short_len: u8 = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let len = short_len as usize;

        let mut result = Vec::with_capacity(len);
        for i in 0..len {
            let elem = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(i, &self))?;
            result.push(elem);
        }
        Ok(result)
    }
}

/// If you don't want to use the ShortVec newtype, you can do ShortVec
/// deserialization on an ordinary vector with the following field annotation:
///
/// #[serde(with = "short_vec")]
///
pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let visitor = ShortVecVisitor { _t: PhantomData };
    deserializer.deserialize_tuple(std::u16::MAX as usize, visitor)
}

pub struct ShortVec<T>(pub Vec<T>);

impl<T: Serialize> Serialize for ShortVec<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(&self.0, serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ShortVec<T> {
    fn deserialize<D>(deserializer: D) -> Result<ShortVec<T>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize(deserializer).map(ShortVec)
    }
}

/// Return the decoded value and how many bytes it consumed.
pub fn decode_len(bytes: &[u8]) -> (usize, usize) {
    let short_len: u8 = bincode::deserialize(bytes).unwrap();
    let num_bytes = bincode::serialized_size(&short_len).unwrap() as u16;
    (short_len as usize, num_bytes as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::{deserialize, serialize};

    /// Return the serialized length.
    fn encode_len(len: u8) -> Vec<u8> {
        bincode::serialize(&len).unwrap()
    }

    fn assert_len_encoding(len: u8, bytes: &[u8]) {
        assert_eq!(encode_len(len), bytes, "unexpected u8 encoding");
        assert_eq!(
            decode_len(bytes),
            (len as usize, bytes.len()),
            "unexpected u16 decoding"
        );
    }

    #[test]
    fn test_short_vec_encode_len() {
        assert_len_encoding(0x0, &[0x0]);
        assert_len_encoding(0x7f, &[0x7f]);
        assert_len_encoding(0x80, &[0x80]);
        assert_len_encoding(0xff, &[0xff]);
    }

    #[test]
    #[should_panic]
    fn test_short_vec_decode_zero_len() {
        decode_len(&[]);
    }

    #[test]
    fn test_short_vec_u8() {
        let vec = ShortVec(vec![4u8; 32]);
        let bytes = serialize(&vec).unwrap();
        assert_eq!(bytes.len(), vec.0.len() + 1);

        let vec1: ShortVec<u8> = deserialize(&bytes).unwrap();
        assert_eq!(vec.0, vec1.0);
    }

    #[test]
    fn test_short_vec_json() {
        let vec = ShortVec(vec![0, 1, 2]);
        let s = serde_json::to_string(&vec).unwrap();
        assert_eq!(s, "[3,0,1,2]");
    }
}
