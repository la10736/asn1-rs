use crate::ber::*;
use crate::der_constraint_fail_if;
use crate::error::*;
use crate::DynTagged;
use crate::ToDer;
use crate::ToStatic;
use crate::{FromBer, FromDer};
use nom::bytes::streaming::take;
use rusticata_macros::newtype_enum;
use std::borrow::Cow;
use std::convert::TryFrom;
use std::fmt;
use std::ops;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BerClassFromIntError(pub(crate) ());

/// BER Object class of tag
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum Class {
    Universal = 0b00,
    Application = 0b01,
    ContextSpecific = 0b10,
    Private = 0b11,
}

/// BER/DER Tag as defined in X.680 section 8.4
///
/// X.690 doesn't specify the maximum tag size so we're assuming that people
/// aren't going to need anything more than a u32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag(pub u32);

newtype_enum! {
impl display Tag {
    EndOfContent = 0x0,
    Boolean = 0x1,
    Integer = 0x2,
    BitString = 0x3,
    OctetString = 0x4,
    Null = 0x05,
    Oid = 0x06,
    ObjDescriptor = 0x07,
    External = 0x08,
    RealType = 0x09,
    Enumerated = 0xa,
    EmbeddedPdv = 0xb,
    Utf8String = 0xc,
    RelativeOid = 0xd,

    Sequence = 0x10,
    Set = 0x11,
    NumericString = 0x12,
    PrintableString = 0x13,
    T61String = 0x14,
    TeletexString = 0x14,
    VideotexString = 0x15,

    Ia5String = 0x16,
    UtcTime = 0x17,
    GeneralizedTime = 0x18,

    GraphicString = 25, // 0x19
    VisibleString = 26, // 0x1a
    GeneralString = 27, // 0x1b

    UniversalString = 0x1c,
    BmpString = 0x1e,

    Invalid = 0xff,
}
}

/// BER Object Length
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Length {
    /// Definite form (X.690 8.1.3.3)
    Definite(usize),
    /// Indefinite form (X.690 8.1.3.6)
    Indefinite,
}

/// BER/DER object header (identifier and length)
#[derive(Clone, Debug)]
pub struct Header<'a> {
    /// Object class: universal, application, context-specific, or private
    pub(crate) class: Class,
    /// Constructed attribute: true if constructed, else false
    pub(crate) constructed: bool,
    /// Tag number
    pub(crate) tag: Tag,
    /// Object length: value if definite, or indefinite
    pub(crate) length: Length,

    /// Optionally, the raw encoding of the tag
    ///
    /// This is useful in some cases, where different representations of the same
    /// BER tags have different meanings (BER only)
    pub(crate) raw_tag: Option<Cow<'a, [u8]>>,
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Class::Universal => "UNIVERSAL",
            Class::Application => "APPLICATION",
            Class::ContextSpecific => "CONTEXT-SPECIFIC",
            Class::Private => "PRIVATE",
        };
        write!(f, "{}", s)
    }
}

impl TryFrom<u8> for Class {
    type Error = BerClassFromIntError;

    #[inline]
    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0b00 => Ok(Class::Universal),
            0b01 => Ok(Class::Application),
            0b10 => Ok(Class::ContextSpecific),
            0b11 => Ok(Class::Private),
            _ => Err(BerClassFromIntError(())),
        }
    }
}

impl Tag {
    pub const fn assert_eq(&self, tag: Tag) -> Result<()> {
        if self.0 == tag.0 {
            Ok(())
        } else {
            Err(Error::UnexpectedTag {
                expected: Some(tag),
                actual: *self,
            })
        }
    }

    pub fn invalid_value(&self, msg: &str) -> Error {
        Error::InvalidValue {
            tag: *self,
            msg: msg.to_string(),
        }
    }
}

impl From<u32> for Tag {
    fn from(v: u32) -> Self {
        Tag(v)
    }
}

impl Length {
    /// Return true if length is definite and equal to 0
    #[inline]
    pub fn is_null(&self) -> bool {
        *self == Length::Definite(0)
    }

    /// Get length of primitive object
    #[inline]
    pub fn definite(&self) -> Result<usize> {
        match self {
            Length::Definite(sz) => Ok(*sz),
            Length::Indefinite => Err(Error::IndefiniteLengthUnexpected),
        }
    }

    /// Return true if length is definite
    #[inline]
    pub const fn is_definite(&self) -> bool {
        matches!(self, Length::Definite(_))
    }

    /// Return error if length is not definite
    #[inline]
    pub const fn assert_definite(&self) -> Result<()> {
        match self {
            Length::Definite(_) => Ok(()),
            Length::Indefinite => Err(Error::IndefiniteLengthUnexpected),
        }
    }
}

impl From<usize> for Length {
    fn from(l: usize) -> Self {
        Length::Definite(l)
    }
}

impl ops::Add<Length> for Length {
    type Output = Self;

    fn add(self, rhs: Length) -> Self::Output {
        match self {
            Length::Indefinite => self,
            Length::Definite(lhs) => match rhs {
                Length::Indefinite => self,
                Length::Definite(rhs) => Length::Definite(lhs + rhs),
            },
        }
    }
}

impl ops::Add<usize> for Length {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        match self {
            Length::Definite(lhs) => Length::Definite(lhs + rhs),
            Length::Indefinite => self,
        }
    }
}

impl ops::AddAssign<usize> for Length {
    fn add_assign(&mut self, rhs: usize) {
        match self {
            Length::Definite(ref mut lhs) => *lhs += rhs,
            Length::Indefinite => (),
        }
    }
}

impl<'a> Header<'a> {
    /// Build a new BER/DER header from the provided values
    pub const fn new(class: Class, constructed: bool, tag: Tag, length: Length) -> Self {
        Header {
            tag,
            constructed,
            class,
            length,
            raw_tag: None,
        }
    }

    /// Build a new BER/DER header from the provided tag, with default values for other fields
    #[inline]
    pub const fn new_simple(tag: Tag) -> Self {
        let constructed = matches!(tag, Tag::Sequence | Tag::Set);
        Self::new(Class::Universal, constructed, tag, Length::Definite(0))
    }

    /// Set the class of this `Header`
    #[inline]
    pub fn with_class(self, class: Class) -> Self {
        Self { class, ..self }
    }

    /// Set the constructed flags of this `Header`
    #[inline]
    pub fn with_constructed(self, constructed: bool) -> Self {
        Self {
            constructed,
            ..self
        }
    }

    /// Set the tag of this `Header`
    #[inline]
    pub fn with_tag(self, tag: Tag) -> Self {
        Self { tag, ..self }
    }

    /// Set the length of this `Header`
    #[inline]
    pub fn with_lenth(self, length: Length) -> Self {
        Self { length, ..self }
    }

    /// Update header to add reference to raw tag
    #[inline]
    pub fn with_raw_tag(self, raw_tag: Option<Cow<'a, [u8]>>) -> Self {
        Header { raw_tag, ..self }
    }

    /// Return the class of this header.
    #[inline]
    pub const fn class(&self) -> Class {
        self.class
    }

    /// Return true if this header has the 'constructed' flag.
    #[inline]
    pub const fn constructed(&self) -> bool {
        self.constructed
    }

    /// Return the tag of this header.
    #[inline]
    pub const fn tag(&self) -> Tag {
        self.tag
    }

    /// Return the length of this header.
    #[inline]
    pub const fn length(&self) -> Length {
        self.length
    }

    /// Test if object is primitive
    #[inline]
    pub const fn is_primitive(&self) -> bool {
        !self.constructed
    }

    /// Test if object is constructed
    #[inline]
    pub const fn is_constructed(&self) -> bool {
        self.constructed
    }

    /// Return error if class is not the expected class
    #[inline]
    pub const fn assert_class(&self, class: Class) -> Result<()> {
        if self.class as u8 == class as u8 {
            Ok(())
        } else {
            Err(Error::UnexpectedClass(class))
        }
    }

    /// Return error if tag is not the expected tag
    #[inline]
    pub const fn assert_tag(&self, tag: Tag) -> Result<()> {
        self.tag.assert_eq(tag)
    }

    /// Return error if object is not primitive
    #[inline]
    pub const fn assert_primitive(&self) -> Result<()> {
        if self.is_primitive() {
            Ok(())
        } else {
            Err(Error::ConstructUnexpected)
        }
    }

    /// Return error if object is primitive
    #[inline]
    pub const fn assert_constructed(&self) -> Result<()> {
        if !self.is_primitive() {
            Ok(())
        } else {
            Err(Error::ConstructExpected)
        }
    }
}

impl<'a> ToStatic for Header<'a> {
    type Owned = Header<'static>;

    fn to_static(&self) -> Self::Owned {
        let raw_tag: Option<Cow<'static, [u8]>> =
            self.raw_tag.as_ref().map(|b| Cow::Owned(b.to_vec()));
        Header {
            tag: self.tag,
            constructed: self.constructed,
            class: self.class,
            length: self.length,
            raw_tag,
        }
    }
}

impl<'a> FromBer<'a> for Header<'a> {
    fn from_ber(bytes: &'a [u8]) -> ParseResult<Self> {
        let (i1, el) = parse_identifier(bytes)?;
        let class = match Class::try_from(el.0) {
            Ok(c) => c,
            Err(_) => unreachable!(), // Cannot fail, we have read exactly 2 bits
        };
        let (i2, len) = parse_ber_length_byte(i1)?;
        let (i3, len) = match (len.0, len.1) {
            (0, l1) => {
                // Short form: MSB is 0, the rest encodes the length (which can be 0) (8.1.3.4)
                (i2, Length::Definite(usize::from(l1)))
            }
            (_, 0) => {
                // Indefinite form: MSB is 1, the rest is 0 (8.1.3.6)
                // If encoding is primitive, definite form shall be used (8.1.3.2)
                if el.1 == 0 {
                    return Err(nom::Err::Error(Error::ConstructExpected));
                }
                (i2, Length::Indefinite)
            }
            (_, l1) => {
                // if len is 0xff -> error (8.1.3.5)
                if l1 == 0b0111_1111 {
                    return Err(::nom::Err::Error(Error::InvalidLength));
                }
                let (i3, llen) = take(l1)(i2)?;
                match bytes_to_u64(llen) {
                    Ok(l) => {
                        let l =
                            usize::try_from(l).or(Err(::nom::Err::Error(Error::InvalidLength)))?;
                        (i3, Length::Definite(l))
                    }
                    Err(_) => {
                        return Err(::nom::Err::Error(Error::InvalidLength));
                    }
                }
            }
        };
        let constructed = el.1 != 0;
        let hdr = Header::new(class, constructed, Tag(el.2), len).with_raw_tag(Some(el.3.into()));
        Ok((i3, hdr))
    }
}

impl<'a> FromDer<'a> for Header<'a> {
    fn from_der(bytes: &'a [u8]) -> ParseResult<Self> {
        let (i1, el) = parse_identifier(bytes)?;
        let class = match Class::try_from(el.0) {
            Ok(c) => c,
            Err(_) => unreachable!(), // Cannot fail, we have read exactly 2 bits
        };
        let (i2, len) = parse_ber_length_byte(i1)?;
        let (i3, len) = match (len.0, len.1) {
            (0, l1) => {
                // Short form: MSB is 0, the rest encodes the length (which can be 0) (8.1.3.4)
                (i2, Length::Definite(usize::from(l1)))
            }
            (_, 0) => {
                // Indefinite form is not allowed in DER (10.1)
                return Err(::nom::Err::Error(Error::IndefiniteLengthUnexpected));
            }
            (_, l1) => {
                // if len is 0xff -> error (8.1.3.5)
                if l1 == 0b0111_1111 {
                    return Err(::nom::Err::Error(Error::InvalidLength));
                }
                // DER(9.1) if len is 0 (indefinite form), obj must be constructed
                der_constraint_fail_if!(&i[1..], len.1 == 0 && el.1 != 1);
                let (i3, llen) = take(l1)(i2)?;
                match bytes_to_u64(llen) {
                    Ok(l) => {
                        // DER: should have been encoded in short form (< 127)
                        // XXX der_constraint_fail_if!(i, l < 127);
                        let l =
                            usize::try_from(l).or(Err(::nom::Err::Error(Error::InvalidLength)))?;
                        (i3, Length::Definite(l))
                    }
                    Err(_) => {
                        return Err(::nom::Err::Error(Error::InvalidLength));
                    }
                }
            }
        };
        let constructed = el.1 != 0;
        let hdr = Header::new(class, constructed, Tag(el.2), len).with_raw_tag(Some(el.3.into()));
        Ok((i3, hdr))
    }
}

impl DynTagged for (Class, bool, Tag) {
    fn tag(&self) -> Tag {
        self.2
    }
}

impl ToDer for (Class, bool, Tag) {
    fn to_der_len(&self) -> Result<usize> {
        let (_, _, tag) = self;
        match tag.0 {
            0..=30 => Ok(1),
            t => {
                let mut sz = 1;
                let mut val = t;
                loop {
                    if val <= 127 {
                        return Ok(sz + 1);
                    } else {
                        val >>= 7;
                        sz += 1;
                    }
                }
            }
        }
    }

    fn write_der_header(&self, writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        let (class, constructed, tag) = self;
        let b0 = (*class as u8) << 6;
        let b0 = b0 | if *constructed { 0b10_0000 } else { 0 };
        if tag.0 > 30 {
            let b0 = b0 | 0b1_1111;
            let mut sz = writer.write(&[b0])?;
            let mut val = tag.0;
            loop {
                if val <= 127 {
                    sz += writer.write(&[val as u8])?;
                    return Ok(sz);
                } else {
                    let b = (val & 0b0111_1111) as u8 | 0b1000_0000;
                    sz += writer.write(&[b])?;
                    val >>= 7;
                }
            }
        } else {
            let b0 = b0 | (tag.0 as u8);
            let sz = writer.write(&[b0])?;
            Ok(sz)
        }
    }

    fn write_der_content(&self, _writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        Ok(0)
    }
}

impl DynTagged for Length {
    fn tag(&self) -> Tag {
        Tag(0)
    }
}

impl ToDer for Length {
    fn to_der_len(&self) -> Result<usize> {
        match self {
            Length::Indefinite => Ok(1),
            Length::Definite(l) => match l {
                0..=0x7f => Ok(1),
                0x80..=0xff => Ok(2),
                0x100..=0x7fff => Ok(3),
                0x8000..=0xffff => Ok(4),
                _ => Err(Error::InvalidLength),
            },
        }
    }

    fn write_der_header(&self, writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        match *self {
            Length::Indefinite => {
                let sz = writer.write(&[0b1000_0000])?;
                Ok(sz)
            }
            Length::Definite(l) => {
                if l <= 127 {
                    // Short form
                    let sz = writer.write(&[l as u8])?;
                    Ok(sz)
                } else {
                    // Long form
                    let mut sz = 0;
                    let mut val = l;
                    loop {
                        if val <= 127 {
                            sz += writer.write(&[val as u8])?;
                            return Ok(sz);
                        } else {
                            let b = (val & 0b0111_1111) as u8 | 0b1000_0000;
                            sz += writer.write(&[b])?;
                            val >>= 7;
                        }
                    }
                }
            }
        }
    }

    fn write_der_content(&self, _writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        Ok(0)
    }
}

impl DynTagged for Header<'_> {
    fn tag(&self) -> Tag {
        self.tag
    }
}

impl ToDer for Header<'_> {
    fn to_der_len(&self) -> Result<usize> {
        let tag_len = (self.class, self.constructed, self.tag).to_der_len()?;
        let len_len = self.length.to_der_len()?;
        Ok(tag_len + len_len)
    }

    fn write_der_header(&self, writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        let sz = (self.class, self.constructed, self.tag).write_der_header(writer)?;
        let sz = sz + self.length.write_der_header(writer)?;
        Ok(sz)
    }

    fn write_der_content(&self, _writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        Ok(0)
    }

    fn write_der_raw(&self, writer: &mut dyn std::io::Write) -> SerializeResult<usize> {
        // use raw_tag if present
        let sz = match &self.raw_tag {
            Some(t) => writer.write(&t)?,
            None => (self.class, self.constructed, self.tag).write_der_header(writer)?,
        };
        let sz = sz + self.length.write_der_header(writer)?;
        Ok(sz)
    }
}

/// Compare two BER headers. `len` fields are compared only if both objects have it set (same for `raw_tag`)
impl<'a> PartialEq<Header<'a>> for Header<'a> {
    fn eq(&self, other: &Header) -> bool {
        self.class == other.class
            && self.tag == other.tag
            && self.constructed == other.constructed
            && {
                if self.length.is_null() && other.length.is_null() {
                    self.length == other.length
                } else {
                    true
                }
            }
            && {
                // it tag is present for both, compare it
                if self.raw_tag.as_ref().xor(other.raw_tag.as_ref()).is_none() {
                    self.raw_tag == other.raw_tag
                } else {
                    true
                }
            }
    }
}

impl Eq for Header<'_> {}
