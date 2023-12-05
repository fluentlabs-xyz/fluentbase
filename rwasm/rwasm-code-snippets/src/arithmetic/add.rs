use crate::consts::{U64_HALF_BITS_COUNT, U64_LOW_PART_MASK};

#[no_mangle]
pub fn arithmetic_add(
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> (u64, u64, u64, u64) {
    let mut a_part: u64 = 0;
    let mut b_part: u64 = 0;
    let mut part_sum: u64 = 0;
    let mut carry: u64 = 0;
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    a_part = a0 & U64_LOW_PART_MASK;
    b_part = b0 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part;
    s0 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a0 >> U64_HALF_BITS_COUNT;
    b_part = b0 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s0 = s0 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a1 & U64_LOW_PART_MASK;
    b_part = b1 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s1 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a1 >> U64_HALF_BITS_COUNT;
    b_part = b1 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s1 = s1 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a2 & U64_LOW_PART_MASK;
    b_part = b2 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s2 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a2 >> U64_HALF_BITS_COUNT;
    b_part = b2 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s2 = s2 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);
    carry = part_sum >> U64_HALF_BITS_COUNT;

    a_part = a3 & U64_LOW_PART_MASK;
    b_part = b3 & U64_LOW_PART_MASK;
    part_sum = a_part + b_part + carry;
    s3 = part_sum & U64_LOW_PART_MASK;
    carry = part_sum >> U64_HALF_BITS_COUNT;
    a_part = a3 >> U64_HALF_BITS_COUNT;
    b_part = b3 >> U64_HALF_BITS_COUNT;
    part_sum = a_part + b_part + carry;
    s3 = s3 + ((part_sum & U64_LOW_PART_MASK) << U64_HALF_BITS_COUNT);

    (s0, s1, s2, s3)
}
