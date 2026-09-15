//! Static trace metadata. Descriptors are registered during bootstrap and hot
//! paths retain only their numeric [`DescriptorId`].

/// Producer-local descriptor index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DescriptorId(pub u32);

/// Producer-local category index. Capture masks support 256 categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CategoryId(pub u8);

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "collector", derive(serde::Serialize, serde::Deserialize))]
pub enum DescriptorKind {
    Instant = 1,
    Span = 2,
    AsyncSpan = 3,
    Flow = 4,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "collector", derive(serde::Serialize, serde::Deserialize))]
pub enum ArgumentType {
    None = 0,
    Unsigned = 1,
    Signed = 2,
    FloatBits = 3,
    Boolean = 4,
    Identifier = 5,
    Bytes = 6,
    Duration = 7,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "collector", derive(serde::Serialize, serde::Deserialize))]
pub enum Unit {
    None = 0,
    Count = 1,
    Bytes = 2,
    Nanoseconds = 3,
    Microseconds = 4,
    Milliseconds = 5,
    Frames = 6,
    Samples = 7,
    Percent = 8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "collector", derive(serde::Serialize, serde::Deserialize))]
pub enum Severity {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warning = 3,
    Error = 4,
    Fatal = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentDescriptor<'a> {
    pub name: &'a str,
    pub kind: ArgumentType,
    pub unit: Unit,
}

impl<'a> ArgumentDescriptor<'a> {
    pub const NONE: Self = Self {
        name: "",
        kind: ArgumentType::None,
        unit: Unit::None,
    };

    pub const fn new(name: &'a str, kind: ArgumentType, unit: Unit) -> Self {
        Self { name, kind, unit }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor<'a> {
    pub category: CategoryId,
    pub category_name: &'a str,
    pub name: &'a str,
    pub kind: DescriptorKind,
    pub argument0: ArgumentDescriptor<'a>,
    pub argument1: ArgumentDescriptor<'a>,
    pub severity: Severity,
}

impl<'a> Descriptor<'a> {
    pub const fn new(
        category: CategoryId,
        category_name: &'a str,
        name: &'a str,
        kind: DescriptorKind,
        argument0: ArgumentDescriptor<'a>,
        argument1: ArgumentDescriptor<'a>,
    ) -> Self {
        Self {
            category,
            category_name,
            name,
            kind,
            argument0,
            argument1,
            severity: Severity::Trace,
        }
    }

    pub const fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}
