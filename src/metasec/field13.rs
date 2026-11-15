//! Field 13 request digest with all three recovered branches.
//!
//! Algorithm parameters and reference vectors come from the GPL-3.0 lineage
//! documented at <https://github.com/sunnysky123/hongguo-web/blob/f458d7455322d4fa98540831477d88429a80b0e5/server/src/core/pure-sign.js>.
//! This is a request primitive, not a complete Medusa header or SDK context.

use md5::{Digest as Md5Digest, Md5};
use sm3::Sm3;

const BRANCH2_SV: [u32; 64] = [
    0xa7ae_fe20,
    0x7149_f1d6,
    0x47e4_ca07,
    0xe9b5_8f67,
    0x93b9_24de,
    0xc614_d0f5,
    0x38af_e0ef,
    0xb2bb_ad73,
    0xe244_44c3,
    0x9d3a_ec9b,
    0xdf7b_37e4,
    0xd8b1_6d40,
    0xf8ac_31b8,
    0x76b9_a90b,
    0x31d8_33ee,
    0x953f_ce64,
    0x3535_95a4,
    0x4609_c13b,
    0x3692_5008,
    0x8c6d_0925,
    0x5df5_c177,
    0x1cfb_f52b,
    0x8a4f_a7f0,
    0x114c_a35e,
    0x8193_f984,
    0x7a7a_8733,
    0x316a_b4d5,
    0x3c20_cfc9,
    0xa6d8_4453,
    0x3a18_500c,
    0x798e_c47a,
    0x97a7_6b28,
    0x66c4_ff96,
    0x5171_6443,
    0x0dd2_fc3b,
    0xb569_6da7,
    0xbbeb_3ac5,
    0x5c53_d204,
    0xd326_08ce,
    0x7279_b9ec,
    0xf418_8ecf,
    0xf7d7_93db,
    0x332c_c491,
    0xab76_ae15,
    0x9beb_e727,
    0x18a0_1384,
    0x5be9_f8a7,
    0x5f90_a754,
    0x39b6_63c0,
    0x3667_3c83,
    0x7c92_f514,
    0x9d7d_94d7,
    0xe2e8_d9aa,
    0x5f7e_9ea9,
    0x7abd_4551,
    0x569e_05da,
    0x40a2_5632,
    0x3df5_a9a5,
    0xbab3_7d80,
    0x4542_86dc,
    0x3f5d_4e78,
    0x03d7_b75d,
    0xb1fe_4af7,
    0xa5ab_26a3,
];
const BRANCH2_ORDERS: [u8; 320] = decode_hex(concat!(
    "0f0704000908030a060b050d0e010c02",
    "0f05080c0009020103070e060b0a0d04",
    "060500070c000a04080f010b0d09020e",
    "060b02050403080101070a000d0c090e",
    "0d070e0f0b0208030c0509010004060a",
    "0d0902060f0b0a040807000c0503010e",
    "0c090f07060f030e020d0405010b0a00",
    "0c050a090e0802040407030f01060b00",
    "0b0f0408020a0700090d06010e03050c",
    "0b060a0508020c03070f0e090d000104",
    "0906080f050800040a0b030d01020c0e",
    "090d0c0604070a03030f00080105020e",
    "01000d0f090a0b0e0402080703060c05",
    "01080a0c0f0905060b000304020e070d",
    "04080f000c0f0e0d0a01060207090503",
    "040205080d0b0a0606000e0f070c0903",
    "0a08040f000b01060d0c0709030e0502",
    "0a070b050f00020e0108030d0c060904",
    "0d070f08050f06040b0a0e0c09000203",
    "0d0c020704010b0e0e08060f09050003",
));
const BRANCH0_IV: [u32; 6] = [
    0xc4a7_8580,
    0xb3c0_fd39,
    0xc58c_5686,
    0xc9aa_3ba7,
    0xf5a7_adf2,
    0x963c_2ed1,
];
const BRANCH0_TT: [u32; 64] = [
    0xebb6_4faf,
    0x07aa_dcc2,
    0xcf31_87bf,
    0xe011_38ff,
    0x6d0b_fcff,
    0x5a30_a3be,
    0xb41a_d638,
    0x3418_0eb8,
    0xf233_eb6f,
    0xb1a5_84cc,
    0xccc3_0dc7,
    0x47d1_db51,
    0xd556_53de,
    0x70a8_4fa1,
    0x5747_3c12,
    0xf76f_0288,
    0x2c07_7f0a,
    0xda0d_cad0,
    0xfbb8_6f6c,
    0xfdc4_cf00,
    0x688a_020d,
    0xe676_c6a6,
    0x8cd6_338b,
    0x1a3c_8d0e,
    0xcce8_b06b,
    0x6ad0_ed0b,
    0xa052_2717,
    0xdc71_ac83,
    0x2285_db71,
    0xd5b4_dda6,
    0x736f_8650,
    0x6560_306c,
    0x617c_e2a6,
    0xe423_417e,
    0x0a40_e143,
    0x544e_4032,
    0x88df_fb2a,
    0x716c_1ae0,
    0x4c46_7a88,
    0x05b2_3bb3,
    0xe1d0_b866,
    0xbaa3_dcb8,
    0xae33_74d3,
    0xc338_1a50,
    0x1702_f75b,
    0xfe6d_a368,
    0xf0b4_cf48,
    0x4e0f_fbb8,
    0x72aa_d10d,
    0x26c5_3a3d,
    0xf2bc_e0f6,
    0xb455_7581,
    0x4a25_7fdd,
    0x8c31_82a2,
    0xab0b_3b86,
    0x3d5d_fb14,
    0x4f10_3634,
    0xd37b_52d7,
    0x444e_ff16,
    0xeb0a_33d1,
    0x6ca8_6f6e,
    0x0028_4ba7,
    0x0838_7cfa,
    0x5fb3_7586,
];
const BRANCH0_INIT: [u32; 8] = [
    0x7aba_4fc8,
    0x6716_6507,
    0x6403_fa00,
    0x340f_512f,
    984_304_912,
    3_005_047_866,
    2_874_125_293,
    2_152_413_264,
];
const BRANCH0_ROUNDS: [[usize; 11]; 6] = [
    [101, 5, 7, 6, 3, 2, 1, 0, 5, 4, 3],
    [96, 0, 6, 7, 5, 3, 2, 1, 5, 4, 4],
    [96, 7, 6, 2, 1, 4, 0, 5, 4, 3, 5],
    [99, 3, 6, 2, 4, 5, 1, 0, 0, 7, 6],
    [96, 0, 5, 6, 7, 3, 1, 2, 5, 4, 4],
    [100, 2, 0, 3, 5, 4, 6, 7, 2, 1, 5],
];

const fn decode_hex<const N: usize>(input: &str) -> [u8; N] {
    let bytes = input.as_bytes();
    assert!(bytes.len() == N * 2, "invalid hexadecimal constant length");
    let mut result = [0; N];
    let mut index = 0;
    while index < N {
        let high = bytes[index * 2];
        let low = bytes[index * 2 + 1];
        result[index] = (hex_nibble(high) << 4) | hex_nibble(low);
        index += 1;
    }
    result
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid hexadecimal constant"),
    }
}

/// Hash raw query and optional body bytes, then return the selected digest branch.
#[must_use]
pub fn branch(query: &[u8], body: Option<&[u8]>, khronos: u32) -> u32 {
    let query_hash = sm3(query);
    let body_hash = body.map(md5).unwrap_or([0; 16]);
    selector(&query_hash, &body_hash, &khronos.to_le_bytes()).0
}

/// Compute field 13 from raw query and optional body bytes.
#[must_use]
pub fn digest(query: &[u8], body: Option<&[u8]>, khronos: u32) -> Vec<u8> {
    digest_hashed(&sm3(query), &body.map(md5).unwrap_or([0; 16]), khronos)
}

/// Compute field 13 from a 32-byte SM3 query hash and 16-byte MD5 body hash.
#[must_use]
pub fn digest_hashed(query: &[u8; 32], body: &[u8; 16], khronos: u32) -> Vec<u8> {
    let timestamp = khronos.to_le_bytes();
    let (branch, variant, iv) = selector(query, body, &timestamp);
    match branch {
        0 => branch_zero(query, body, &timestamp, khronos, variant),
        2 => branch_two(query, body, &timestamp, khronos, iv),
        _ => {
            let mut combined = Vec::with_capacity(48);
            combined.extend_from_slice(query);
            combined.extend_from_slice(body);
            let checksum = checksum(&combined);
            let mut result = Vec::with_capacity(36);
            result.extend_from_slice(&query[..16]);
            result.extend_from_slice(body);
            result.extend_from_slice(&checksum.to_le_bytes());
            result
        }
    }
}

fn selector(query: &[u8], body: &[u8], timestamp: &[u8; 4]) -> (u32, usize, u32) {
    let iv = initial_iv(initial_iv(initial_iv(0x2023_0928, query), body), timestamp);
    let low = iv & 15;
    let variant = ((low * 171) >> 9) as usize;
    (low - (variant as u32) * 3, variant, iv)
}

fn initial_iv(mut value: u32, data: &[u8]) -> u32 {
    for (index, byte) in data.iter().enumerate() {
        value = if index & 1 == 0 {
            (value >> 4) ^ value ^ value.wrapping_shl(6) ^ u32::from(*byte)
        } else {
            !((value >> 7) ^ value ^ (u32::from(*byte) | value.wrapping_shl(12)))
        };
    }
    value
}

fn checksum(data: &[u8]) -> u32 {
    let mut value = 0x2022_0420_u32;
    for (index, byte) in data[..12].iter().enumerate() {
        let previous = if index & 1 == 0 {
            (value >> 3) ^ value
        } else {
            (value >> 5) ^ value
        };
        value = if index & 1 == 0 {
            u32::from(*byte) ^ value.wrapping_shl(7)
        } else {
            u32::from(*byte) | value.wrapping_shl(11)
        };
        if index & 1 == 1 {
            value = !value;
        }
        value ^= previous;
    }
    (value | 4) ^ 0x0100_0000
}

fn branch_zero(
    query: &[u8; 32],
    body: &[u8; 16],
    timestamp: &[u8; 4],
    khronos: u32,
    variant: usize,
) -> Vec<u8> {
    let iv = BRANCH0_IV[variant];
    let rotation = iv.wrapping_add(khronos).wrapping_add(1) & 255;
    let count = iv.wrapping_add(khronos);
    let round_constants = BRANCH0_TT.map(|value| value.rotate_right(rotation));
    let shift = count.wrapping_add(2) & 7;
    let mut input = Vec::with_capacity(64);
    input.extend_from_slice(query);
    input.extend_from_slice(body);
    input.extend_from_slice(timestamp);
    for byte in [0xfa_u8, 0x45, 0x61, 0xd7] {
        let repeated = u16::from(byte) | (u16::from(byte) << 8);
        input.push(((repeated >> shift) & 255) as u8);
    }
    input.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 0xa0]);

    let mut words = Vec::with_capacity(128);
    for chunk in input.as_chunks::<4>().0 {
        words.push(u32::from_be_bytes(*chunk));
    }
    for index in 0..112 {
        let first = words[index + 1];
        let second = words[index + 14];
        let left = first.rotate_left(14) ^ first.rotate_left(25) ^ (first >> 3);
        let right = second.rotate_left(13) ^ second.rotate_left(15) ^ (second >> 10);
        let next = words[index]
            .wrapping_add(words[index + 9])
            .wrapping_add(left)
            .wrapping_add(right);
        words.push(next);
    }

    let initial = BRANCH0_INIT.map(|value| value.rotate_right(count & 31));
    let mut state = initial;
    let rounds = &BRANCH0_ROUNDS[variant];
    for round in 0..rounds[0] {
        let base = iv.wrapping_add(round as u32);
        let word = words[(base & 127) as usize];
        let choose = |index: &usize| state[*index];
        let first =
            ((choose(&rounds[3]) ^ choose(&rounds[4])) & choose(&rounds[1])) ^ choose(&rounds[3]);
        let rotations = choose(&rounds[1]).rotate_left(26)
            ^ choose(&rounds[1]).rotate_left(21)
            ^ choose(&rounds[1]).rotate_left(7);
        let sum = word
            .wrapping_add(first)
            .wrapping_add(rotations)
            .wrapping_add(round_constants[(base & 63) as usize])
            .wrapping_add(choose(&rounds[5]));
        let other_rotations = choose(&rounds[2]).rotate_left(30)
            ^ choose(&rounds[2]).rotate_left(19)
            ^ choose(&rounds[2]).rotate_left(10);
        let boolean = (choose(&rounds[2]) & choose(&rounds[6]))
            | ((choose(&rounds[2]) | choose(&rounds[6])) & choose(&rounds[7]));
        let other = other_rotations.wrapping_add(boolean);
        let old = state[rounds[9]];
        state.rotate_right(1);
        state[rounds[10]] = other.wrapping_add(sum);
        state[rounds[8]] = old.wrapping_add(sum);
    }

    let mut folded = [0; 16];
    for index in 0..8 {
        let word = state[index].wrapping_add(initial[index]).to_be_bytes();
        if index < 4 {
            folded[index * 4..index * 4 + 4].copy_from_slice(&word);
        }
    }
    for index in 4..8 {
        let word = state[index].wrapping_add(initial[index]).to_be_bytes();
        for offset in 0..4 {
            folded[(index - 4) * 4 + offset] ^= word[offset];
        }
    }
    let mut result = Vec::with_capacity(20);
    result.extend_from_slice(&folded);
    result.extend_from_slice(&checksum(&folded).to_le_bytes());
    result
}

fn branch_two(
    query: &[u8; 32],
    body: &[u8; 16],
    timestamp: &[u8; 4],
    khronos: u32,
    iv: u32,
) -> Vec<u8> {
    let mixed = (iv & 13).wrapping_mul(86);
    let variant = (((mixed >> 15) & 255) + ((mixed >> 8) & 255)) as usize;
    let seeds: [u32; 5] = [
        0x8980_f29b,
        0xeb54_9c7f,
        0xb087_26db,
        0xd40c_b5e6,
        0xe8f5_59e4,
    ];
    let seed = seeds[variant];
    let rotation = seed.wrapping_add(khronos).wrapping_add(1) & 255;
    let count = seed.wrapping_add(khronos);
    let shift = count.wrapping_add(5) & 7;
    let mut input = Vec::with_capacity(64);
    input.extend_from_slice(query);
    input.extend_from_slice(body);
    input.extend_from_slice(timestamp);
    for byte in [0x84_u8, 0x96, 0x77, 0x9d, 0xd4, 0x15, 0x0b, 0xf8] {
        let repeated = u16::from(byte) | (u16::from(byte) << 8);
        input.push(((repeated >> shift) & 255) as u8);
    }
    input.extend_from_slice(&[0xa0, 1, 0, 0]);
    let orders = &BRANCH2_ORDERS[variant * 64..variant * 64 + 64];
    branch_two_digest(&input, count, orders, rotation)
}

fn branch_two_digest(input: &[u8], count: u32, orders: &[u8], rotation: u32) -> Vec<u8> {
    let constants = BRANCH2_SV.map(|value| value.rotate_right(rotation));
    let start: [u32; 4] = [0x79e0_f2fb_u32, 0xc8b5_2570, 0xebc2_f8cd, 0x7c10_4d93]
        .map(|value| value.rotate_right(count));
    let end_rotation = count.wrapping_add(6) & 255;
    let end: [u32; 4] = [0x19be_4866_u32, 0xe859_86b4, 0xe19b_326e, 0x71d1_d7d4]
        .map(|value| value.rotate_right(end_rotation));
    let mut words = [0; 16];
    for (word, chunk) in words.iter_mut().zip(input.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*chunk);
    }
    let [mut a, mut b, mut c, mut d] = start;
    let shifts = [
        [7, 12, 17, 22],
        [5, 9, 14, 20],
        [4, 11, 16, 23],
        [6, 10, 15, 21],
    ];
    for (kind, shift) in shifts.iter().enumerate() {
        for index in 0..16 {
            let round = kind * 16 + index;
            let mix = match kind {
                0 => (b & c) | (!b & d),
                1 => (b & d) | (c & !d),
                2 => b ^ c ^ d,
                _ => c ^ (b | !d),
            };
            let sum = a
                .wrapping_add(mix)
                .wrapping_add(words[orders[round] as usize])
                .wrapping_add(constants[round]);
            let next = b.wrapping_add(sum.rotate_left(shift[index & 3]));
            a = d;
            d = c;
            c = b;
            b = next;
        }
    }
    let mut result = Vec::with_capacity(20);
    for (word, (initial, final_xor)) in [a, b, c, d].iter().zip(start.iter().zip(end.iter())) {
        result.extend_from_slice(&(initial.wrapping_add(*word) ^ final_xor).to_le_bytes());
    }
    let sum = checksum(&result);
    result.extend_from_slice(&sum.to_le_bytes());
    result
}

fn md5(input: &[u8]) -> [u8; 16] {
    Md5::digest(input).into()
}

fn sm3(input: &[u8]) -> [u8; 32] {
    let mut hasher = <Sm3 as Default>::default();
    sm3::digest::Digest::update(&mut hasher, input);
    sm3::digest::Digest::finalize(hasher).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sm3_matches_the_standard_vector() {
        assert_eq!(
            hex(&sm3(b"abc")),
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
    }

    #[test]
    fn all_branches_match_public_golden_vectors() {
        let vectors = [
            (6, 0, "45900f1eedc492bb5057fbe3c16dbd578ca0f1f9"),
            (10, 0, "107e9784312cfbda0abefda050b807e15e771b9c"),
            (11, 0, "414dc581955817c76eb7d2711db6f76427012ef2"),
            (
                0,
                1,
                "6c672c941166dc5a92051173e82d3ad4301251acb61102bf3ce58237e5dabb2f5ccc5be6",
            ),
            (
                2,
                1,
                "a4ca6efd792be1e9272bfbfdd7db063376af63c5bd77b2a3a2cfcbd52645fa38ed085890",
            ),
            (
                4,
                1,
                "160a86344b4d0dc1d3477a00c7b924c80039fc0a2fa335c82bfd08f6a068ee1cb506705f",
            ),
            (1, 2, "9afd2d7f22283226358ce80c967381996e237834"),
            (3, 2, "3b98a30f2373f0b81f5d572add6fe6396ff1a622"),
            (7, 2, "847690989f21c1663e0bdaeae26affc9f4cf28d0"),
        ];
        for (index, expected_branch, expected) in vectors {
            let query = format!("a=1&t={index}");
            let body = format!("body{index}");
            let timestamp = 1_700_000_000 + index;
            assert_eq!(
                branch(query.as_bytes(), Some(body.as_bytes()), timestamp),
                expected_branch
            );
            assert_eq!(
                hex(&digest(query.as_bytes(), Some(body.as_bytes()), timestamp)),
                expected,
                "branch {expected_branch}, index {index}"
            );
        }
    }

    #[test]
    fn absent_body_uses_the_zero_digest_and_changes_field13() {
        let timestamp = 1_700_000_006;
        assert_ne!(
            digest(b"a=1&t=6", None, timestamp),
            digest(b"a=1&t=6", Some(b"body6"), timestamp)
        );
        assert_eq!(digest(b"a=1&t=6", None, timestamp).len(), 20);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
