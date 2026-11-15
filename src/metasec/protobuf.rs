//! Minimal protobuf encoder for signing messages.
//! It intentionally supports only field types observed in the recovered messages.

/// A borrowed field value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    Bytes(&'a [u8]),
    Text(&'a str),
    Signed(i64),
    Unsigned(u64),
    Float(f32),
    Message(&'a [u8]),
}

/// A numbered protobuf field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

/// Encode fields in the supplied order without sorting or normalization.
#[must_use]
pub fn encode(fields: &[Field<'_>]) -> Vec<u8> {
    let mut result = Vec::new();
    for field in fields {
        match field.value {
            Value::Text("") => {}
            Value::Bytes(bytes) => encode_bytes(&mut result, field.number, 2, bytes),
            Value::Message(bytes) => encode_bytes(&mut result, field.number, 2, bytes),
            Value::Text(text) => encode_bytes(&mut result, field.number, 2, text.as_bytes()),
            Value::Signed(value) => {
                encode_varint(&mut result, u64::from(field.number) << 3);
                let zigzag = ((value as u64) << 1) ^ ((value >> 63) as u64);
                encode_varint(&mut result, zigzag);
            }
            Value::Unsigned(value) => {
                encode_varint(&mut result, u64::from(field.number) << 3);
                encode_varint(&mut result, value);
            }
            Value::Float(value) => {
                encode_varint(&mut result, (u64::from(field.number) << 3) | 5);
                result.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    result
}

fn encode_bytes(output: &mut Vec<u8>, number: u32, wire: u64, bytes: &[u8]) {
    encode_varint(output, (u64::from(number) << 3) | wire);
    encode_varint(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
}

fn encode_varint(output: &mut Vec<u8>, mut value: u64) {
    while value > 127 {
        output.push(((value & 127) | 128) as u8);
        value >>= 7;
    }
    output.push(value as u8);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_field_order_types_and_nested_messages() {
        let nested = [0x08, 0x02];
        let encoded = encode(&[
            Field {
                number: 1,
                value: Value::Signed(1),
            },
            Field {
                number: 2,
                value: Value::Signed(2),
            },
            Field {
                number: 4,
                value: Value::Text("8662"),
            },
            Field {
                number: 5,
                value: Value::Bytes(&[0, 1]),
            },
            Field {
                number: 8,
                value: Value::Float(1.0),
            },
            Field {
                number: 9,
                value: Value::Message(&nested),
            },
            Field {
                number: 10,
                value: Value::Text(""),
            },
            Field {
                number: 11,
                value: Value::Signed(-1),
            },
        ]);
        assert_eq!(
            encoded,
            [
                0x08, 0x02, 0x10, 0x04, 0x22, 0x04, b'8', b'6', b'6', b'2', 0x2a, 0x02, 0, 1, 0x45,
                0, 0, 0x80, 0x3f, 0x4a, 0x02, 0x08, 0x02, 0x58, 0x01,
            ]
        );
    }
}
