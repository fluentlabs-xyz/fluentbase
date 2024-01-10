use crate::{Byte32, Fr, Hash, HashScheme, HASH_DOMAIN_BYTE32, HASH_DOMAIN_ELEMS_BASE};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use std::prelude::v1::*;
use uint::construct_uint;

construct_uint! {
    pub struct U256(4);
}

pub fn fr_from_usize(value: usize) -> Fr {
    let value = U256::from(value);
    let mut value_bytes = [0u8; 32];
    value.to_little_endian(&mut value_bytes);
    Fr::from_bytes(&value_bytes).unwrap()
}

pub fn fr_from_big_endian(buf: &[u8]) -> Result<Fr, String> {
    let value = if buf.len() == 32 {
        let values = [
            BigEndian::read_u64(&buf[0..8]),
            BigEndian::read_u64(&buf[8..16]),
            BigEndian::read_u64(&buf[16..24]),
            BigEndian::read_u64(&buf[24..32]),
        ];
        Fr::from_raw(values)
    } else {
        let value = U256::from_big_endian(buf);
        Fr::from_raw(value.0)
    };
    Ok(value)
}

pub fn fr_from_little_endian(buf: &[u8]) -> Result<Fr, String> {
    let value = if buf.len() == 32 {
        let values = [
            LittleEndian::read_u64(&buf[0..8]),
            LittleEndian::read_u64(&buf[8..16]),
            LittleEndian::read_u64(&buf[16..24]),
            LittleEndian::read_u64(&buf[24..32]),
        ];
        Fr::from_raw(values)
    } else {
        let value = U256::from_little_endian(buf);
        Fr::from_raw(value.0)
    };
    Ok(value)
}

pub fn fr_to_little_endian(fr: &Fr) -> [u8; 32] {
    fr.to_bytes()
}

pub fn reverse_byte_order(dst: &mut [u8], src: &[u8]) {
    assert_eq!(dst.len(), src.len());
    for i in 0..src.len() {
        dst[src.len() - 1 - i] = src[i]
    }
}

pub fn handling_elems_and_byte32<H: HashScheme>(
    flag_array: u32,
    elems: &[Byte32],
) -> Result<Hash, String> {
    let mut ret = Vec::with_capacity(elems.len());
    for (i, elem) in elems.iter().enumerate() {
        if flag_array & (1 << i) != 0 {
            ret.push(elem.hash::<H>()?);
        } else {
            ret.push(elem.fr()?);
        }
    }

    if ret.len() < 2 {
        return Ok(ret.first().map(|fr| fr.into()).unwrap_or_default());
    }

    Ok(hash_elems::<H>(&ret[0], &ret[1], &ret[2..]))
}

// HashElemsWithDomain performs a recursive poseidon hash over the array of ElemBytes, each hash
// reduce 2 fieds into one, with a specified domain field which would be used in
// every recursiving call
pub fn hash_elems_with_domain<H: HashScheme>(
    domain: &Fr,
    fst: &Fr,
    snd: &Fr,
    elems: &[Fr],
) -> Hash {
    let l = elems.len();
    let base_h = H::hash_scheme(&[*fst, *snd], domain);
    if l == 0 {
        return base_h.into();
    } else if l == 1 {
        return hash_elems_with_domain::<H>(domain, &base_h, &elems[0], &[]);
    }

    let mut tmp = Vec::with_capacity((l + 1) / 2);
    for i in 0..(l + 1) / 2 {
        if (i + 1) * 2 > l {
            tmp.push(elems[i * 2])
        } else {
            tmp.push(H::hash_scheme(&elems[i * 2..(i + 1) * 2], &domain));
        }
    }
    hash_elems_with_domain::<H>(domain, &base_h, &tmp[0], &tmp[1..])
}

// HashElems call HashElemsWithDomain with a domain of HASH_DOMAIN_ELEMS_BASE(256)*<element counts>
pub fn hash_elems<H: HashScheme>(fst: &Fr, snd: &Fr, elems: &[Fr]) -> Hash {
    let domain = elems.len() * HASH_DOMAIN_ELEMS_BASE + HASH_DOMAIN_BYTE32;
    let domain = fr_from_usize(domain);
    hash_elems_with_domain::<H>(&domain, fst, snd, elems)
}

pub fn test_bit(bitmap: &[u8], n: usize) -> bool {
    bitmap[n / 8] & (1 << (n % 8)) != 0
}

pub fn test_bit_big_endian(bitmap: &[u8], n: usize) -> bool {
    bitmap[bitmap.len() - n / 8 - 1] & (1 << (n % 8)) != 0
}

pub fn to_secure_key<H: HashScheme>(key: &[u8]) -> Result<Fr, Error> {
    let word = Byte32::from_bytes_padding(key);
    word.hash::<H>().map_err(Error::NotInField)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    ReachedMaxLevel,
    EntryIndexAlreadyExists,
    NodeKeyAlreadyExists,
    NodeNotFound((usize, Hash)),
    KeyNotFound,
    InvalidField,
    NodeBytesBadSize,
    InvalidNodeFound(u8),
    NotInField(String),
    ExpectedLeafNode,
}
