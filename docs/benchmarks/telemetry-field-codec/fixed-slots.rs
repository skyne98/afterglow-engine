//! Bounded field-codec prototype. It is not part of the active capture ABI.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privacy {
    Public,
    Sensitive,
    Secret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceKind {
    Operation,
    Slice,
    Event,
    Resource,
    Epoch,
    Queue,
    Ticket,
    Snapshot,
    Artifact,
    Track,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind<'a> {
    F64,
    U32,
    I32,
    Bool,
    Text { max_bytes: u32 },
    Enum(&'a [&'a str]),
    Reference(ReferenceKind),
}

#[derive(Clone, Copy, Debug)]
pub struct Field<'a> {
    pub name: &'a str,
    pub kind: FieldKind<'a>,
    pub privacy: Option<Privacy>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub fields: usize,
    pub payload_bytes: usize,
    pub metadata_bytes: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub default_privacy: Privacy,
    pub retain_sensitive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub kind: ReferenceKind,
    pub session: [u32; 4],
    pub producer: u32,
    pub producer_generation: u32,
    pub id: u64,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    Redacted,
    F64(f64),
    U32(u32),
    I32(i32),
    Bool(bool),
    Text(&'a str),
    Enum(u32),
    Reference(Reference),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldError {
    Capacity,
    InvalidSchema,
    InvalidValue,
    InvalidEncoding,
}

struct Slot<'a> {
    kind: FieldKind<'a>,
    start: usize,
    end: usize,
    retained: bool,
}

/// Bootstrap owns this layout. Record encoding and decoding allocate nothing.
pub struct FieldCodec<'a> {
    slots: Vec<Slot<'a>>,
    bytes: usize,
}

impl<'a> FieldCodec<'a> {
    pub fn new(
        fields: &'a [Field<'a>],
        limits: Limits,
        policy: Policy,
    ) -> Result<Self, FieldError> {
        if fields.len() > limits.fields {
            return Err(FieldError::Capacity);
        }
        let mut metadata = 0usize;
        let mut bytes = 0usize;
        // Validate all sizes before layout allocation.
        for (index, field) in fields.iter().enumerate() {
            if field.name.is_empty()
                || fields[..index]
                    .iter()
                    .any(|previous| previous.name == field.name)
            {
                return Err(FieldError::InvalidSchema);
            }
            metadata = metadata
                .checked_add(field.name.len())
                .ok_or(FieldError::Capacity)?;
            if metadata > limits.metadata_bytes {
                return Err(FieldError::Capacity);
            }
            let width = match field.kind {
                FieldKind::F64 => 8,
                FieldKind::U32 | FieldKind::I32 => 4,
                FieldKind::Bool => 1,
                FieldKind::Text { max_bytes } => (max_bytes as usize)
                    .checked_add(4)
                    .ok_or(FieldError::Capacity)?,
                FieldKind::Enum(values) => {
                    if values.is_empty() || values.len() > u32::MAX as usize {
                        return Err(FieldError::InvalidSchema);
                    }
                    for (index, value) in values.iter().enumerate() {
                        if value.is_empty() || values[..index].contains(value) {
                            return Err(FieldError::InvalidSchema);
                        }
                        metadata = metadata
                            .checked_add(value.len())
                            .ok_or(FieldError::Capacity)?;
                        if metadata > limits.metadata_bytes {
                            return Err(FieldError::Capacity);
                        }
                    }
                    4
                }
                FieldKind::Reference(_) => 36,
            };
            bytes = bytes
                .checked_add(1)
                .and_then(|value| value.checked_add(width))
                .ok_or(FieldError::Capacity)?;
            if bytes > limits.payload_bytes || metadata > limits.metadata_bytes {
                return Err(FieldError::Capacity);
            }
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(fields.len())
            .map_err(|_| FieldError::Capacity)?;
        let mut start = 0;
        for field in fields {
            let width = match field.kind {
                FieldKind::F64 => 8,
                FieldKind::U32 | FieldKind::I32 | FieldKind::Enum(_) => 4,
                FieldKind::Bool => 1,
                FieldKind::Text { max_bytes } => 4 + max_bytes as usize,
                FieldKind::Reference(_) => 36,
            };
            let end = start + 1 + width;
            let privacy = field.privacy.unwrap_or(policy.default_privacy);
            let retained = privacy == Privacy::Public
                || (privacy == Privacy::Sensitive && policy.retain_sensitive);
            slots.push(Slot {
                kind: field.kind,
                start,
                end,
                retained,
            });
            start = end;
        }
        Ok(Self { slots, bytes })
    }

    pub fn encoded_len(&self) -> usize {
        self.bytes
    }
    pub fn field_count(&self) -> usize {
        self.slots.len()
    }

    /// Validate the full record before output mutation. Excluded values are not examined.
    pub fn encode_into(
        &self,
        values: &[Value<'_>],
        output: &mut [u8],
    ) -> Result<usize, FieldError> {
        if output.len() < self.bytes {
            return Err(FieldError::Capacity);
        }
        if values.len() != self.slots.len() {
            return Err(FieldError::InvalidValue);
        }
        for (slot, value) in self.slots.iter().zip(values) {
            if slot.retained && !valid_value(slot.kind, *value) {
                return Err(FieldError::InvalidValue);
            }
        }
        output[..self.bytes].fill(0);
        for (slot, value) in self.slots.iter().zip(values) {
            if !slot.retained || *value == Value::Redacted {
                continue;
            }
            output[slot.start] = 1;
            let bytes = &mut output[slot.start + 1..slot.end];
            match *value {
                Value::F64(value) => bytes.copy_from_slice(&value.to_le_bytes()),
                Value::U32(value) | Value::Enum(value) => {
                    bytes.copy_from_slice(&value.to_le_bytes())
                }
                Value::I32(value) => bytes.copy_from_slice(&value.to_le_bytes()),
                Value::Bool(value) => bytes[0] = u8::from(value),
                Value::Text(value) => {
                    bytes[..4].copy_from_slice(&(value.len() as u32).to_le_bytes());
                    bytes[4..4 + value.len()].copy_from_slice(value.as_bytes());
                }
                Value::Reference(value) => {
                    for (index, part) in value.session.iter().enumerate() {
                        put_u32(bytes, index * 4, *part);
                    }
                    put_u32(bytes, 16, value.producer);
                    put_u32(bytes, 20, value.producer_generation);
                    bytes[24..32].copy_from_slice(&value.id.to_le_bytes());
                    put_u32(bytes, 32, value.generation);
                }
                Value::Redacted => unreachable!("Retained values were validated"),
            }
        }
        Ok(self.bytes)
    }

    /// Decoded strings borrow the input. Invalid input does not change output values.
    pub fn decode_into<'b>(
        &self,
        input: &'b [u8],
        output: &mut [Value<'b>],
    ) -> Result<usize, FieldError> {
        if input.len() != self.bytes {
            return Err(FieldError::InvalidEncoding);
        }
        if output.len() < self.slots.len() {
            return Err(FieldError::Capacity);
        }
        for slot in &self.slots {
            read_value(slot, input)?;
        }
        for (slot, output) in self.slots.iter().zip(output) {
            *output = read_value(slot, input)?;
        }
        Ok(self.slots.len())
    }
}

fn valid_value(kind: FieldKind<'_>, value: Value<'_>) -> bool {
    match (kind, value) {
        (_, Value::Redacted) => true,
        (FieldKind::F64, Value::F64(value)) => value.is_finite(),
        (FieldKind::U32, Value::U32(_))
        | (FieldKind::I32, Value::I32(_))
        | (FieldKind::Bool, Value::Bool(_)) => true,
        (FieldKind::Text { max_bytes }, Value::Text(value)) => value.len() <= max_bytes as usize,
        (FieldKind::Enum(values), Value::Enum(value)) => (value as usize) < values.len(),
        (FieldKind::Reference(kind), Value::Reference(value)) => {
            value.kind == kind
                && value.session != [0; 4]
                && value.producer_generation != 0
                && value.generation != 0
        }
        _ => false,
    }
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_value<'a>(slot: &Slot<'_>, input: &'a [u8]) -> Result<Value<'a>, FieldError> {
    let bytes = &input[slot.start + 1..slot.end];
    match input[slot.start] {
        0 if bytes.iter().all(|byte| *byte == 0) => return Ok(Value::Redacted),
        1 if slot.retained => {}
        _ => return Err(FieldError::InvalidEncoding),
    }
    let value = match slot.kind {
        FieldKind::F64 => Value::F64(f64::from_le_bytes(bytes.try_into().unwrap())),
        FieldKind::U32 => Value::U32(u32_at(bytes, 0)),
        FieldKind::I32 => Value::I32(i32::from_le_bytes(bytes.try_into().unwrap())),
        FieldKind::Bool => match bytes[0] {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            _ => return Err(FieldError::InvalidEncoding),
        },
        FieldKind::Text { .. } => {
            let length = u32_at(bytes, 0) as usize;
            if length > bytes.len() - 4 || bytes[4 + length..].iter().any(|byte| *byte != 0) {
                return Err(FieldError::InvalidEncoding);
            }
            Value::Text(
                core::str::from_utf8(&bytes[4..4 + length])
                    .map_err(|_| FieldError::InvalidEncoding)?,
            )
        }
        FieldKind::Enum(_) => Value::Enum(u32_at(bytes, 0)),
        FieldKind::Reference(kind) => Value::Reference(Reference {
            kind,
            session: [
                u32_at(bytes, 0),
                u32_at(bytes, 4),
                u32_at(bytes, 8),
                u32_at(bytes, 12),
            ],
            producer: u32_at(bytes, 16),
            producer_generation: u32_at(bytes, 20),
            id: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            generation: u32_at(bytes, 32),
        }),
    };
    if !valid_value(slot.kind, value) {
        return Err(FieldError::InvalidEncoding);
    }
    Ok(value)
}
