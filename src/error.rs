use std::borrow::Borrow;
use std::borrow::Cow;
use std::error;
use std::error::Error as _;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FmtResult;
use std::io;
use std::mem::transmute;
use std::ops::Deref;


mod private {
    pub trait Sealed {}

    impl<T> Sealed for Option<T> {}
    impl<T, E> Sealed for Result<T, E> {}

    impl Sealed for super::Error {}
}

/// A `str` replacement whose owned representation is a `Box<str>` and
/// not a `String`.
#[derive(Debug)]
#[repr(transparent)]
struct Str(str);

impl ToOwned for Str {
    type Owned = Box<str>;

    #[inline]
    fn to_owned(&self) -> Self::Owned {
        self.0.to_string().into_boxed_str()
    }
}

impl Borrow<Str> for Box<str> {
    #[inline]
    fn borrow(&self) -> &Str {
        // SAFETY: `Str` is `repr(transparent)` and so `&str` and `&Str`
        //         can trivially be converted into each other.
        unsafe { transmute::<&str, &Str>(self.deref()) }
    }
}

impl Deref for Str {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// For convenient use in `format!`, for example.
impl Display for Str {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Display::fmt(&self.0, f)
    }
}


// TODO: We may want to support optionally storing a backtrace in
//       terminal variants.
enum ErrorImpl {
    // We don't store `gimli::Error` objects here, because the type is
    // rather useless on its own. To make sense of it you'd need the
    // `gimli::Dwarf` instance in all but trivial cases and it's simply
    // not feasible for us to format the error in a generic way. So we
    // force proper stringification at the call site instead.
    // TODO: Remove allowance once used.
    #[allow(unused)]
    Dwarf(Cow<'static, Str>),
    Io(io::Error),
    // Unfortunately, if we just had a single `Context` variant that
    // contains a `Cow`, this inner `Cow` would cause an overall enum
    // size increase by a machine word, because currently `rustc`
    // seemingly does not fold the necessary bits into the outer enum.
    // We have two variants to work around that until `rustc` is smart
    // enough.
    ContextOwned {
        context: Box<str>,
        source: Box<ErrorImpl>,
    },
    ContextStatic {
        context: &'static str,
        source: Box<ErrorImpl>,
    },
}

impl ErrorImpl {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Dwarf(..) => ErrorKind::InvalidDwarf,
            Self::Io(error) => match error.kind() {
                io::ErrorKind::NotFound => ErrorKind::NotFound,
                io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
                io::ErrorKind::AlreadyExists => ErrorKind::AlreadyExists,
                io::ErrorKind::WouldBlock => ErrorKind::WouldBlock,
                io::ErrorKind::InvalidInput => ErrorKind::InvalidInput,
                io::ErrorKind::InvalidData => ErrorKind::InvalidData,
                io::ErrorKind::TimedOut => ErrorKind::TimedOut,
                io::ErrorKind::WriteZero => ErrorKind::WriteZero,
                io::ErrorKind::Unsupported => ErrorKind::Unsupported,
                io::ErrorKind::UnexpectedEof => ErrorKind::UnexpectedEof,
                io::ErrorKind::OutOfMemory => ErrorKind::OutOfMemory,
                _ => ErrorKind::Other,
            },
            Self::ContextOwned { source, .. } | Self::ContextStatic { source, .. } => {
                source.deref().kind()
            }
        }
    }
}

impl Debug for ErrorImpl {
    // We try to mirror roughly how anyhow's Error is behaving, because
    // that makes the most sense.
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        if f.alternate() {
            let mut dbg;

            match self {
                Self::Dwarf(dwarf) => {
                    dbg = f.debug_tuple(stringify!(Dwarf));
                    dbg.field(dwarf)
                }
                Self::Io(io) => {
                    dbg = f.debug_tuple(stringify!(Io));
                    dbg.field(io)
                }
                Self::ContextOwned { context, .. } => {
                    dbg = f.debug_tuple(stringify!(Context));
                    dbg.field(context)
                }
                Self::ContextStatic { context, .. } => {
                    dbg = f.debug_tuple(stringify!(Context));
                    dbg.field(context)
                }
            }
            .finish()
        } else {
            let () = match self {
                Self::Dwarf(error) => write!(f, "Error: {error}")?,
                Self::Io(error) => write!(f, "Error: {error}")?,
                Self::ContextOwned { context, .. } => write!(f, "Error: {context}")?,
                Self::ContextStatic { context, .. } => write!(f, "Error: {context}")?,
            };

            if let Some(source) = self.source() {
                let () = f.write_str("\n\nCaused by:")?;

                let mut error = Some(source);
                while let Some(err) = error {
                    let () = write!(f, "\n    {err:}")?;
                    error = err.source();
                }
            }
            Ok(())
        }
    }
}

impl Display for ErrorImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let () = match self {
            Self::Dwarf(error) => Display::fmt(error, f)?,
            Self::Io(error) => Display::fmt(error, f)?,
            Self::ContextOwned { context, .. } => Display::fmt(context, f)?,
            Self::ContextStatic { context, .. } => Display::fmt(context, f)?,
        };

        if f.alternate() {
            let mut error = self.source();
            while let Some(err) = error {
                let () = write!(f, ": {err}")?;
                error = err.source();
            }
        }
        Ok(())
    }
}

impl error::Error for ErrorImpl {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Dwarf(..) => None,
            Self::Io(error) => error.source(),
            Self::ContextOwned { source, .. } | Self::ContextStatic { source, .. } => Some(source),
        }
    }
}


/// An enum providing a rough classification of errors.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// An entity was not found, often a file.
    NotFound,
    /// The operation lacked the necessary privileges to complete.
    PermissionDenied,
    /// An entity already exists, often a file.
    AlreadyExists,
    /// The operation needs to block to complete, but the blocking
    /// operation was requested to not occur.
    WouldBlock,
    /// A parameter was incorrect.
    InvalidInput,
    /// Data not valid for the operation were encountered.
    InvalidData,
    /// DWARF input data was invalid.
    InvalidDwarf,
    /// The I/O operation's timeout expired, causing it to be canceled.
    TimedOut,
    /// An error returned when an operation could not be completed
    /// because a call to [`write`] returned [`Ok(0)`].
    WriteZero,
    /// This operation is unsupported on this platform.
    Unsupported,
    /// An error returned when an operation could not be completed
    /// because an "end of file" was reached prematurely.
    UnexpectedEof,
    /// An operation could not be completed, because it failed
    /// to allocate enough memory.
    OutOfMemory,
    /// A custom error that does not fall under any other I/O error
    /// kind.
    Other,
}


/// The error type used by the library.
// Representation is optimized for fast copying (a single machine word),
// not so much for fast creation (as it is heap allocated). We generally
// expect errors to be exceptional, though a lot of functionality is
// fallible (i.e., returns a `Result<T, Error>` which would be penalized
// by a large `Err` variant).
#[repr(transparent)]
pub struct Error {
    /// The top-most error of the chain.
    error: Box<ErrorImpl>,
}

impl Error {
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    /// Layer the provided context on top of this `Error`, creating a
    /// new one in the process.
    fn layer_context(self, context: Cow<'static, Str>) -> Self {
        match context {
            Cow::Owned(context) => Self {
                error: Box::new(ErrorImpl::ContextOwned {
                    context,
                    source: self.error,
                }),
            },
            Cow::Borrowed(context) => Self {
                error: Box::new(ErrorImpl::ContextStatic {
                    context,
                    source: self.error,
                }),
            },
        }
    }
}

impl Debug for Error {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Debug::fmt(&self.error, f)
    }
}

impl Display for Error {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        Display::fmt(&self.error, f)
    }
}

impl error::Error for Error {
    #[inline]
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.error.source()
    }
}

impl From<io::Error> for Error {
    fn from(other: io::Error) -> Self {
        Self {
            error: Box::new(ErrorImpl::Io(other)),
        }
    }
}


pub trait ErrorExt<T>: private::Sealed {
    // If we had specialization of sorts we could be more lenient as to
    // what we can accept, but for now this method always works with
    // static strings and nothing else.
    fn context(self, context: &'static str) -> T;

    fn with_context<C, F>(self, f: F) -> T
    where
        C: ToString,
        F: FnOnce() -> C;
}

impl ErrorExt<Error> for Error {
    fn context(self, context: &'static str) -> Error {
        // SAFETY: `Str` is `repr(transparent)` and so `&str` and `&Str`
        //         can trivially be converted into each other.
        let context = unsafe { transmute::<&str, &Str>(context) };
        self.layer_context(Cow::Borrowed(context))
    }

    fn with_context<C, F>(self, f: F) -> Error
    where
        C: ToString,
        F: FnOnce() -> C,
    {
        let context = f().to_string().into_boxed_str();
        self.layer_context(Cow::Owned(context))
    }
}

impl<T> ErrorExt<Result<T, Error>> for Result<T, Error> {
    fn context(self, context: &'static str) -> Result<T, Error> {
        match self {
            ok @ Ok(..) => ok,
            Err(err) => Err(err.context(context)),
        }
    }

    fn with_context<C, F>(self, f: F) -> Result<T, Error>
    where
        C: ToString,
        F: FnOnce() -> C,
    {
        match self {
            ok @ Ok(..) => ok,
            Err(err) => Err(err.with_context(f)),
        }
    }
}


/// A trait providing conversion shortcuts for creating `Error`
/// instances.
pub trait IntoError<T>: private::Sealed
where
    Self: Sized,
{
    fn ok_or_error<C, F>(self, kind: io::ErrorKind, f: F) -> Result<T, Error>
    where
        C: ToString,
        F: FnOnce() -> C;

    #[inline]
    fn ok_or_invalid_data<C, F>(self, f: F) -> Result<T, Error>
    where
        C: ToString,
        F: FnOnce() -> C,
    {
        self.ok_or_error(io::ErrorKind::InvalidData, f)
    }

    #[inline]
    fn ok_or_invalid_input<C, F>(self, f: F) -> Result<T, Error>
    where
        C: ToString,
        F: FnOnce() -> C,
    {
        self.ok_or_error(io::ErrorKind::InvalidInput, f)
    }
}

impl<T> IntoError<T> for Option<T> {
    #[inline]
    fn ok_or_error<C, F>(self, kind: io::ErrorKind, f: F) -> Result<T, Error>
    where
        C: ToString,
        F: FnOnce() -> C,
    {
        self.ok_or_else(|| Error::from(io::Error::new(kind, f().to_string())))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    use std::mem::size_of;

    use test_log::test;


    /// Check various features of our `Str` wrapper type.
    #[test]
    fn str_wrapper() {
        let b = "test string".to_string().into_boxed_str();
        let s: &Str = b.borrow();
        let _b: Box<str> = s.to_owned();

        assert_eq!(s.to_string(), b.deref());
        assert_eq!(format!("{s:?}"), "Str(\"test string\")");
    }

    /// Check that our `Error` type's size is as expected.
    #[test]
    fn error_size() {
        assert_eq!(size_of::<Error>(), size_of::<usize>());
        assert_eq!(size_of::<ErrorImpl>(), 4 * size_of::<usize>());
    }

    /// Check that we can format errors as expected.
    #[test]
    fn error_formatting() {
        let err = io::Error::new(io::ErrorKind::InvalidData, "some invalid data");
        let err = Error::from(err);

        let src = err.source();
        assert!(src.is_none(), "{src:?}");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(format!("{err}"), "some invalid data");
        assert_eq!(format!("{err:#}"), "some invalid data");
        assert_eq!(format!("{err:?}"), "Error: some invalid data");
        // TODO: The inner format may not actually be all that stable.
        let expected = r#"Io(
    Custom {
        kind: InvalidData,
        error: "some invalid data",
    },
)"#;
        assert_eq!(format!("{err:#?}"), expected);

        let err = err.context("inner context");
        let src = err.source();
        assert!(src.is_some(), "{src:?}");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(format!("{err}"), "inner context");
        assert_eq!(format!("{err:#}"), "inner context: some invalid data");

        let expected = r#"Error: inner context

Caused by:
    some invalid data"#;
        assert_eq!(format!("{err:?}"), expected);
        // Nope, not going to bother.
        assert_ne!(format!("{err:#?}"), "");

        let err = err.with_context(|| "outer context".to_string());
        let src = err.source();
        assert!(src.is_some(), "{src:?}");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert_eq!(format!("{err}"), "outer context");
        assert_eq!(
            format!("{err:#}"),
            "outer context: inner context: some invalid data"
        );

        let expected = r#"Error: outer context

Caused by:
    inner context
    some invalid data"#;
        assert_eq!(format!("{err:?}"), expected);
        assert_ne!(format!("{err:#?}"), "");
    }
}