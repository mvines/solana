/// This module provides a hybrid base91-encoding for some types used in
/// bucket storage.  Numbers are split up into 13 bit chunks encoded as
/// two base91 characters with a / between each.
///
/// Examples:
///     * 0u64              - "  /  /  /  /  "
///     * u64::MAX          - "P /~!/~!/~!/~!"
///     * Pubkey::default() - "  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  "
///
/// Requirements that motivated this encoding:
///
/// 1. For all types this module supports it's assumed that:
///    * the values to be encoded will be randomly distributed across the
///      entire type range
///    * there will be billions of values to encode
///
/// 2. While bucket storage doesn't care about the number of objects in a
///    bucket, the filesystem of most developer machines start to slow down
///    significantly when a directory contains more than 10,000 files.  The
///    encoding therefore adds a directory separator (`/`) after every second
///    base91 digit to limit the number of files per directory to 91^2 (8281).
///
/// 3. The encoding for u64 values must have the same sort order as the u64
///    values themselves.  Bucket storage object listing is in lexical order.
///
/// 4. Bucket object names may not contain the NUL or / character
///
/// 5. The pattern "/.." is not permitted in the encoded output
///

use solana_sdk::{
    pubkey::Pubkey,
    signature::Signature,
};
use std::convert::TryInto;

// Note: This character set excludes '/' and '.', but does include characters like
// *?[] that are likely to require escaping when used with shell globbing and regex patterns
const BASE91_CHARSET: [u8;91]= *b" !\"#$%&'()*+,-0123456789;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[]^_`abcdefghijklmnopqrstuvwxyz{|}~";

pub fn encode_u64(mut value: u64) -> String {
    let mut s = *b"\0\0/\0\0/\0\0/\0\0/\0\0";

    for i in [12, 9, 6, 3, 0].iter() {
        let b = (value & 0x1FFF) as usize;
        value >>= 13;

        s[*i] = BASE91_CHARSET[b / 91];
        s[*i + 1] = BASE91_CHARSET[b % 91];
    }

    String::from_utf8(s.to_vec()).unwrap()
}

pub fn encode_signature(signature: &Signature) -> String {
    let bytes = signature.as_ref();

    (0..8).map(|i|
        encode_u64(u64::from_le_bytes(bytes[i*8..(i+1)*8].try_into().unwrap()))
        ).collect::<Vec<_>>().join("/")
}

pub fn encode_pubkey(pubkey: &Pubkey) -> String {
    let bytes = pubkey.as_ref();

    (0..4).map(|i|
        encode_u64(u64::from_le_bytes(bytes[i*8..(i+1)*8].try_into().unwrap()))
        ).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_encode_u64() {
        assert!(dbg!(encode_u64(0)) < dbg!(encode_u64(std::u64::MAX)));
        assert!(encode_u64(std::u64::MAX-1) < encode_u64(std::u64::MAX));

        for i in 0..0xffff {
            assert!(encode_u64(i) < encode_u64(i + 1));
        }
    }

    #[test]
    fn test_encode_signature() {
            assert_eq!(encode_signature(&Signature::default()), "  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  ");
    }

    #[test]
    fn test_encode_pubkey() {
            assert_eq!(encode_pubkey(&Pubkey::default()), "  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  /  ");
            assert_eq!(encode_pubkey(&solana_sdk::bpf_loader::id()), "4x/([/Z9/O3/8S/I0/;d/g2/F[/*R/#^/ Z/!v/o0/^8/  /  / F/%o/=<");
    }
}
