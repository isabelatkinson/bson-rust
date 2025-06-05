use std::{
    borrow::{Borrow, Cow},
    convert::{TryFrom, TryInto},
    iter::FromIterator,
    ops::Deref,
};

use serde::{Deserialize, Serialize};

use crate::{de::MIN_BSON_DOCUMENT_SIZE, Document};

use super::{
    bson::RawBson,
    iter::Iter,
    serde::OwnedOrBorrowedRawDocument,
    Error,
    RawBsonRef,
    RawDocument,
    RawIter,
    Result,
};

mod raw_writer;

/// An owned BSON document (akin to [`std::path::PathBuf`]), backed by a buffer of raw BSON bytes.
/// This can be created from a `Vec<u8>` or a [`crate::Document`].
///
/// Accessing elements within a [`RawDocumentBuf`] is similar to element access in
/// [`crate::Document`], but because the contents are parsed during iteration instead of at creation
/// time, format errors can happen at any time during use.
///
/// Iterating over a [`RawDocumentBuf`] yields either an error or a key-value pair that borrows from
/// the original document without making any additional allocations.
///
/// ```
/// # use bson::error::Error;
/// use bson::raw::RawDocumentBuf;
///
/// let doc = RawDocumentBuf::from_bytes(b"\x13\x00\x00\x00\x02hi\x00\x06\x00\x00\x00y'all\x00\x00".to_vec())?;
/// let mut iter = doc.iter();
/// let (key, value) = iter.next().unwrap()?;
/// assert_eq!(key, "hi");
/// assert_eq!(value.as_str(), Some("y'all"));
/// assert!(iter.next().is_none());
/// # Ok::<(), Error>(())
/// ```
///
/// This type implements [`Deref`] to [`RawDocument`], meaning that all methods on [`RawDocument`]
/// are available on [`RawDocumentBuf`] values as well. This includes [`RawDocument::get`] or any of
/// the type-specific getters, such as [`RawDocument::get_object_id`] or [`RawDocument::get_str`].
/// Note that accessing elements is an O(N) operation, as it requires iterating through the document
/// from the beginning to find the requested key.
///
/// ```
/// use bson::raw::RawDocumentBuf;
///
/// let doc = RawDocumentBuf::from_bytes(b"\x13\x00\x00\x00\x02hi\x00\x06\x00\x00\x00y'all\x00\x00".to_vec())?;
/// assert_eq!(doc.get_str("hi")?, "y'all");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, PartialEq)]
pub struct RawDocumentBuf {
    data: Vec<u8>,
}

impl RawDocumentBuf {
    /// Creates a new, empty [`RawDocumentBuf`].
    pub fn new() -> RawDocumentBuf {
        let mut data = Vec::new();
        data.extend(MIN_BSON_DOCUMENT_SIZE.to_le_bytes());
        data.push(0);
        Self { data }
    }

    /// Constructs a new [`RawDocumentBuf`], validating _only_ the
    /// following invariants:
    ///   * `data` is at least five bytes long (the minimum for a valid BSON document)
    ///   * the initial four bytes of `data` accurately represent the length of the bytes as
    ///     required by the BSON spec.
    ///   * the last byte of `data` is a 0
    ///
    /// Note that the internal structure of the bytes representing the
    /// BSON elements is _not_ validated at all by this method. If the
    /// bytes do not conform to the BSON spec, then method calls on
    /// the RawDocument will return Errors where appropriate.
    ///
    /// ```
    /// # use bson::raw::RawDocumentBuf;
    /// let doc = RawDocumentBuf::from_bytes(b"\x05\0\0\0\0".to_vec())?;
    /// # Ok::<(), bson::error::Error>(())
    /// ```
    pub fn from_bytes(data: Vec<u8>) -> Result<RawDocumentBuf> {
        let _ = RawDocument::from_bytes(data.as_slice())?;
        Ok(Self { data })
    }

    /// Create a [`RawDocumentBuf`] from a [`Document`].
    ///
    /// ```
    /// use bson::{doc, oid::ObjectId, raw::RawDocumentBuf};
    ///
    /// let document = doc! {
    ///     "_id": ObjectId::new(),
    ///     "name": "Herman Melville",
    ///     "title": "Moby-Dick",
    /// };
    /// let doc = RawDocumentBuf::from_document(&document)?;
    /// # Ok::<(), bson::error::Error>(())
    /// ```
    pub fn from_document(doc: &Document) -> Result<RawDocumentBuf> {
        let mut data = Vec::new();
        doc.to_writer(&mut data).map_err(Error::malformed_value)?;

        Ok(Self { data })
    }

    /// Gets an iterator over the elements in the [`RawDocumentBuf`], which yields
    /// `Result<(&str, RawBson<'_>)>`.
    ///
    /// ```
    /// use bson::{doc, raw::RawDocumentBuf};
    ///
    /// let doc = RawDocumentBuf::from_document(&doc! { "ferris": true })?;
    ///
    /// for element in doc.iter() {
    ///     let (key, value) = element?;
    ///     assert_eq!(key, "ferris");
    ///     assert_eq!(value.as_bool(), Some(true));
    /// }
    /// # Ok::<(), bson::error::Error>(())
    /// ```
    ///
    /// # Note:
    ///
    /// There is no owning iterator for [`RawDocumentBuf`]. If you need ownership over
    /// elements that might need to allocate, you must explicitly convert
    /// them to owned types yourself.
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(self)
    }

    /// Gets an iterator over the elements in the [`RawDocumentBuf`],
    /// which yields `Result<RawElement<'_>>` values. These hold a
    /// reference to the underlying document but do not explicitly
    /// resolve the values.
    ///
    /// This iterator, which underpins the implementation of the
    /// default iterator, produces `RawElement` objects that hold a
    /// view onto the document but do not parse out or construct
    /// values until the `.value()` or `.try_into()` methods are
    /// called.
    ///
    /// # Note:
    ///
    /// There is no owning iterator for [`RawDocumentBuf`]. If you
    /// need ownership over elements that might need to allocate, you
    /// must explicitly convert them to owned types yourself.
    pub fn iter_elements(&self) -> RawIter<'_> {
        RawIter::new(self)
    }

    /// Return the contained data as a `Vec<u8>`
    ///
    /// ```
    /// use bson::{doc, raw::RawDocumentBuf};
    ///
    /// let doc = RawDocumentBuf::from_document(&doc!{})?;
    /// assert_eq!(doc.into_bytes(), b"\x05\x00\x00\x00\x00".to_vec());
    /// # Ok::<(), bson::error::Error>(())
    /// ```
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Append a key value pair to the end of the document without checking to see if
    /// the key already exists.
    ///
    /// It is a user error to append the same key more than once to the same document, and it may
    /// result in errors when communicating with MongoDB.
    ///
    /// If the provided key contains an interior null byte, this method will panic.
    ///
    /// Values can be any type that can be converted to either borrowed or owned raw bson data; see
    /// the documentation for [BindRawBsonRef] for more details.
    /// ```
    /// # use bson::error::Error;
    /// use bson::{doc, raw::{RawBsonRef, RawDocumentBuf}};
    ///
    /// let mut doc = RawDocumentBuf::new();
    /// // `&str` and `i32` both convert to `RawBsonRef`
    /// doc.append("a string", "some string");
    /// doc.append("an integer", 12_i32);
    ///
    /// let mut subdoc = RawDocumentBuf::new();
    /// subdoc.append("a key", true);
    /// doc.append("a borrowed document", &subdoc);
    /// doc.append("an owned document", subdoc);
    ///
    /// let expected = doc! {
    ///     "a string": "some string",
    ///     "an integer": 12_i32,
    ///     "a borrowed document": { "a key": true },
    ///     "an owned document": { "a key": true },
    /// };
    ///
    /// assert_eq!(doc.to_document()?, expected);
    /// # Ok::<(), Error>(())
    /// ```
    pub fn append(&mut self, key: impl AsRef<str>, value: impl BindRawBsonRef) {
        value.bind(|value_ref| {
            raw_writer::RawWriter::new(&mut self.data)
                .append(key.as_ref(), value_ref)
                .expect("key should not contain interior null byte")
        })
    }

    /// Convert this [`RawDocumentBuf`] to a [`Document`], returning an error
    /// if invalid BSON is encountered.
    pub fn to_document(&self) -> Result<Document> {
        self.as_ref().try_into()
    }
}

impl Default for RawDocumentBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for RawDocumentBuf {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(OwnedOrBorrowedRawDocument::deserialize(deserializer)?.into_owned())
    }
}

impl Serialize for RawDocumentBuf {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let doc: &RawDocument = self.deref();
        doc.serialize(serializer)
    }
}

impl std::fmt::Debug for RawDocumentBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawDocumentBuf")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl From<RawDocumentBuf> for Cow<'_, RawDocument> {
    fn from(rd: RawDocumentBuf) -> Self {
        Cow::Owned(rd)
    }
}

impl<'a> From<&'a RawDocumentBuf> for Cow<'a, RawDocument> {
    fn from(rd: &'a RawDocumentBuf) -> Self {
        Cow::Borrowed(rd.as_ref())
    }
}

impl TryFrom<RawDocumentBuf> for Document {
    type Error = Error;

    fn try_from(raw: RawDocumentBuf) -> Result<Document> {
        Document::try_from(raw.as_ref())
    }
}

impl TryFrom<&Document> for RawDocumentBuf {
    type Error = Error;

    fn try_from(doc: &Document) -> Result<RawDocumentBuf> {
        RawDocumentBuf::from_document(doc)
    }
}

impl<'a> IntoIterator for &'a RawDocumentBuf {
    type IntoIter = Iter<'a>;
    type Item = Result<(&'a str, RawBsonRef<'a>)>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl AsRef<RawDocument> for RawDocumentBuf {
    fn as_ref(&self) -> &RawDocument {
        RawDocument::new_unchecked(&self.data)
    }
}

impl Deref for RawDocumentBuf {
    type Target = RawDocument;

    fn deref(&self) -> &Self::Target {
        RawDocument::new_unchecked(&self.data)
    }
}

impl Borrow<RawDocument> for RawDocumentBuf {
    fn borrow(&self) -> &RawDocument {
        self.deref()
    }
}

impl<S: AsRef<str>, T: BindRawBsonRef> FromIterator<(S, T)> for RawDocumentBuf {
    fn from_iter<I: IntoIterator<Item = (S, T)>>(iter: I) -> Self {
        let mut buf = RawDocumentBuf::new();
        for (k, v) in iter {
            buf.append(k, v);
        }
        buf
    }
}

/// Types that can be consumed to produce raw bson references valid for a limited lifetime.
/// Conceptually a union between `T: Into<RawBson>` and `T: Into<RawBsonRef>`; if your type
/// implements `Into<RawBsonRef>` it will automatically implement this, but if it only
/// implements `Into<RawBson>` it will need to manually define the trivial impl.
pub trait BindRawBsonRef {
    fn bind<F, R>(self, f: F) -> R
    where
        F: for<'a> FnOnce(RawBsonRef<'a>) -> R;
}

impl<'a, T: Into<RawBsonRef<'a>>> BindRawBsonRef for T {
    fn bind<F, R>(self, f: F) -> R
    where
        F: for<'b> FnOnce(RawBsonRef<'b>) -> R,
    {
        f(self.into())
    }
}

impl BindRawBsonRef for RawBson {
    fn bind<F, R>(self, f: F) -> R
    where
        F: for<'a> FnOnce(RawBsonRef<'a>) -> R,
    {
        f(self.as_raw_bson_ref())
    }
}

impl BindRawBsonRef for &RawBson {
    fn bind<F, R>(self, f: F) -> R
    where
        F: for<'a> FnOnce(RawBsonRef<'a>) -> R,
    {
        f(self.as_raw_bson_ref())
    }
}

macro_rules! raw_bson_from_impls {
    ($($t:ty),+$(,)?) => {
        $(
            impl BindRawBsonRef for $t {
                fn bind<F, R>(self, f: F) -> R
                where
                    F: for<'a> FnOnce(RawBsonRef<'a>) -> R,
                {
                    let tmp: RawBson = self.into();
                    f(tmp.as_raw_bson_ref())
                }
            }
        )+
    };
}

raw_bson_from_impls! {
    &crate::binary::Vector,
    crate::Binary,
    crate::DbPointer,
    super::RawArrayBuf,
    RawDocumentBuf,
    super::RawJavaScriptCodeWithScope,
    crate::Regex,
    String,
    crate::binary::Vector,
}
