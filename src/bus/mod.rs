extern crate utf8_cstr;

use ffi;
use ffi::{c_int, c_char, c_void};
use std::{fmt,str};
use std::ffi::CStr;
use std::os::unix::io::AsRawFd;
use std::mem::{uninitialized, transmute, forget};
use std::ptr;
use std::ops::{Deref,DerefMut};
use std::marker::PhantomData;
use std::borrow::{Borrow,BorrowMut};
use std::result;
use self::utf8_cstr::Utf8CStr;

pub mod types;

/**
 * Result type for dbus calls that contains errors returned by remote services (and local errors as
 * well).
 *
 * For functions that can pass over dbus, sd-bus provides detailed error information for all
 * failures, including those cause by bus failures (not necessarily errors sent by the called
 * method).
 *
 * To clairfy: getting this error does not necessarily mean it comes from a remote service. It
 * might be a local failure (resource exaustion, programmer error, service unreachable) as well.
 */
pub type Result<T> = result::Result<T, Error>;

/**
 * A wrapper which promises it always holds a valid dbus object path
 *
 * Requirements (from dbus spec 0.26):
 *
 * - path must begin with ASCII '/' and consist of elements separated by slash characters
 * - each element must only contain the ASCII characters '[A-Z][a-z][0-9]_'
 * - No element may be the empty string
 * - Multiple '/' characters may not occur in sequence
 * - A trailing '/' character is not allowed unless the path is the root path
 * - Further, sd-bus additionally requires nul ('\0') termination of paths.
 */
#[derive(Debug)]
pub struct ObjectPath {
    inner: CStr,
}

impl ObjectPath {
    /**
     * Create a path reference from a u8 slice. Performs all checking needed to ensure requirements
     * are met.
     */
    pub fn from_bytes(b: &[u8]) -> result::Result<&ObjectPath, &'static str> {
        if b.len() < 1 {
            return Err("Path must have at least 1 character ('/')");
        }

        if b[0] != b'/' as u8 {
            return Err("Path must begin with '/'");
        }

        for w in b.windows(2) {
            let prev = w[0];
            let c = w[1];

            match c {
                b'/' => {
                    if prev == b'/' {
                        return Err("Path must not have 2 '/' next to each other");
                    }
                }
                b'A'...b'Z' | b'a'...b'z' | b'0'...b'9' | b'_' => {
                    // Ok
                }
                b'\0' => {
                    if prev == b'/' && b.len() != 2 {
                        return Err("Path must not end in '/' unless it is the root path");
                    }

                    return Ok(unsafe { ObjectPath::from_bytes_unchecked(b) });
                }
                _ => {
                    return Err("Invalid character in path, only '[A-Z][a-z][0-9]_/' allowed");
                }
            }
        }

        return Err("Path must be terminated in a '\\0' byte (for use by sd-bus)");
    }

    #[inline]
    pub unsafe fn from_bytes_unchecked(b: &[u8]) -> &ObjectPath {
        transmute(b)
    }

    #[inline]
    pub unsafe fn from_ptr_unchecked<'b>(b: *const c_char) -> &'b ObjectPath {
       Self::from_bytes_unchecked(CStr::from_ptr(b).to_bytes())
    }
}

impl Deref for ObjectPath {
    type Target = CStr;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[test]
fn t_path() {
    ObjectPath::from_bytes(b"/\0").unwrap();
    ObjectPath::from_bytes(b"\0").err().unwrap();
    ObjectPath::from_bytes(b"/").err().unwrap();
    ObjectPath::from_bytes(b"/h\0").unwrap();
    ObjectPath::from_bytes(b"/hello\0").unwrap();
    ObjectPath::from_bytes(b"/hello/\0").err().unwrap();
    ObjectPath::from_bytes(b"/hello/goodbye/013/4/HA\0").unwrap();
    ObjectPath::from_bytes(b"/hello/goodbye/013/4?/HA\0").err().unwrap();
}

/**
 * A wrapper which promises it always holds a validated dbus interface name
 */
#[derive(Debug)]
pub struct InterfaceName {
    inner: CStr,
}

impl InterfaceName {
    /**
     * Create a interface name reference from a u8 slice.
     *
     * Users should be careful to ensure all the following
     * requirements are met:
     *
     * dbus spec 0.26 requires:
     *  composed of 1 or more elements seperated by a period ('.') character.
     *  Elements contain at least 1 character
     *  Elements must contain only the ASCII characters '[A-Z][a-z][0-9]_' and must not begin with
     *    a digit
     *  Interface names must contain at least one '.' character (and thus at least 2 elements)
     *  Interface names must not being with a '.' character
     * sd-bus additionally requires nul ('\0') termination of the interface name.
     */
    pub fn from_bytes(b: &[u8]) -> result::Result<&InterfaceName, &'static str> {

        if b.len() < 1 {
            return Err("Name must have more than 0 characters");
        }

        match b[0] {
            b'.' => return Err("Name must not begin with '.'"),
            b'A'...b'Z' | b'a'...b'z' | b'_' => {
                // Ok
            }
            _ => return Err("Name must only begin with '[A-Z][a-z]_'"),
        }


        let mut periods = 0;
        for w in b.windows(2) {
            let prev = w[0];
            let c = w[1];
            match c {
                b'.' => {
                    if prev == b'.' {
                        return Err("Name must not have 2 '.' next to each other");
                    }

                    periods += 1;
                }
                b'A'...b'Z' | b'a'...b'z' | b'_' => {
                    // Ok
                }
                b'0'...b'9' => {
                    if prev == b'.' {
                        return Err("Name element must not start with '[0-9]'");
                    }
                    // otherwise, Ok
                }
                b'\0' => {
                    if prev == b'.' && b.len() != 1 {
                        return Err("Name must not end in '.'");
                    }

                    if periods < 1 {
                        return Err("Name must have at least 2 elements");
                    }
                    return Ok(unsafe { InterfaceName::from_bytes_unchecked(b) });
                }
                _ => {
                    return Err("Invalid character in interface name, only '[A-Z][a-z][0-9]_\\.' \
                                allowed");
                }
            }
        }

        return Err("Name must be terminated in a '\\0' byte (for use by sd-bus)");
    }

    /// Unsafety:
    ///
    ///  - `b` must be a nul terminated string
    ///  - `b` must contain a valid interface
    #[inline]
    pub unsafe fn from_bytes_unchecked(b: &[u8]) -> &InterfaceName {
        transmute(b)
    }

    /// Unsafety:
    ///
    ///  - lifetime `'a` must be valid
    ///  - `b` must be a nul terminated string
    ///  - `b` must contain a valid interface
    #[inline]
    pub unsafe fn from_ptr_unchecked<'a>(b: *const c_char) -> &'a Self {
        Self::from_bytes_unchecked(CStr::from_ptr(b).to_bytes_with_nul())
    }
}

impl Deref for InterfaceName {
    type Target = CStr;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}


#[test]
fn t_interface() {
    InterfaceName::from_bytes(b"12\0").err().unwrap();
    InterfaceName::from_bytes(b"a\0").err().unwrap();
    InterfaceName::from_bytes(b"a.b\0").unwrap();
    InterfaceName::from_bytes(b"a.b.3\0").err().unwrap();
    InterfaceName::from_bytes(b"A.Z.xar.yfds.d3490\0").unwrap();
    InterfaceName::from_bytes(b"a.b.c\0").unwrap();
    InterfaceName::from_bytes(b"a.b.c?\0").err().unwrap();
}

#[derive(Debug)]
pub struct BusName {
    inner: CStr,
}

impl BusName {
    /**
     * Create a bus name reference from a u8 slice.
     *
     * Users should be careful to ensure all the following
     * requirements are met:
     *
     * dbus spec 0.26 requires:
     *  unique names start with a ':'. well-known names do not.
     *  composed of one or more elemenets seperated by a period '.'
     *  all elements must be at least 1 character
     *  elements can contain only the ASCII characters '[A-Z][a-z][0-9]_-'.
     *  elements part of a unique name may begin with a digit. elements in all other bus names must
     *    not begin with a digit.
     *  must contain at least 1 '.', and thus at least 2 elements
     *  must not begin with '.'
     *  must be less than the maximum name length (255)
     *
     * sd-bus additionally requires nul ('\0') termination of the bus name.
     */
    pub fn from_bytes(b: &[u8]) -> result::Result<&Self, &'static str> {

        if b.len() < 1 {
            return Err("Name must have more than 0 characters");
        }

        if b.len() > 256 {
            return Err("Must be shorter than 255 characters");
        }

        let mut is_unique = false;
        match b[0] {
            b'.' => return Err("Name must not begin with '.'"),
            b'A'...b'Z' | b'a'...b'z' | b'_' | b'-' => {
                // Ok
            }
            b':' => {
                is_unique = true; /* Ok */
            }
            _ => return Err("Name must only begin with '[A-Z][a-z]_'"),
        }

        let mut periods = 0;
        for w in b.windows(2) {
            let prev = w[0];
            let c = w[1];
            match c {
                b'.' => {
                    if prev == b'.' || prev == b':' {
                        return Err("Elements may not be empty");
                    }

                    periods += 1;
                }
                b'A'...b'Z' | b'a'...b'z' | b'_' | b'-' => {
                    // Ok
                }
                b'0'...b'9' => {
                    if prev == b'.' && !is_unique {
                        return Err("Name element must not start with '[0-9]'");
                    }
                    // otherwise, Ok
                }
                b'\0' => {
                    if prev == b'.' && b.len() != 1 {
                        return Err("Name must not end in '.'");
                    }

                    if periods < 1 {
                        return Err("Name must have at least 2 elements");
                    }
                    return Ok(unsafe { BusName::from_bytes_unchecked(b) });
                }
                _ => {
                    return Err("Invalid character in bus name, only '[A-Z][a-z][0-9]_\\.' allowed");
                }
            }
        }

        return Err("Name must be terminated in a '\\0' byte (for use by sd-bus)");
    }

    #[inline]
    pub unsafe fn from_bytes_unchecked(b: &[u8]) -> &Self {
        transmute(b)
    }

    #[inline]
    pub unsafe fn from_ptr_unchecked<'a>(b: *const c_char) -> &'a Self {
        Self::from_bytes_unchecked(CStr::from_ptr(b).to_bytes())
    }
}

impl Deref for BusName {
    type Target = CStr;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[test]
fn t_busname() {
    BusName::from_bytes(b"a.b\0").unwrap();
    BusName::from_bytes(b"a.b").err().unwrap();
    BusName::from_bytes(b"a\0").err().unwrap();
    BusName::from_bytes(b"a.b?\0").err().unwrap();
    BusName::from_bytes(b"a.b-c.a0\0").unwrap();
    BusName::from_bytes(b"a.b-c.0a\0").err().unwrap();
    BusName::from_bytes(b":a.b-c\0").unwrap();
    BusName::from_bytes(b":a.b-c.1\0").unwrap();
}

#[derive(Debug)]
pub struct MemberName {
    inner: CStr,
}

impl MemberName {
    /**
     * Create a member name reference from a u8 slice.
     *
     * Users should be careful to ensure all the following
     * requirements are met:
     *
     * dbus spec 0.26 requires:
     *  must only contain the ASCII characters '[A-Z][a-z][0-9]_' and may not begin with a digit
     *  must not contain the '.' character
     *  must not exceed the maximum name length (255)
     *  must be at least 1 byte in length
     *
     * sd-bus additionally requires nul ('\0') termination of the bus name.
     */
    pub fn from_bytes(b: &[u8]) -> result::Result<&Self, &'static str> {

        if b.len() < 2 {
            return Err("Name must have more than 0 characters");
        }

        if b.len() > 256 {
            return Err("Must be shorter than 255 characters");
        }

        match b[0] {
            b'A'...b'Z' | b'a'...b'z' | b'_' => {
                // Ok
            }
            _ => return Err("Must begin with '[A-Z][a-z]_'"),
        }

        for c in b {
            match *c {
                b'A'...b'Z' | b'a'...b'z' | b'0'...b'9' | b'_' => {
                    // Ok
                }
                b'\0' => return Ok(unsafe { Self::from_bytes_unchecked(b) }),
                _ => {
                    return Err("Invalid character in member name, only '[A-Z][a-z][0-9]_' allowed");
                }
            }
        }

        return Err("Name must be terminated in a '\\0' byte (for use by sd-bus)");
    }

    #[inline]
    pub unsafe fn from_bytes_unchecked(b: &[u8]) -> &Self {
        transmute(b)
    }

    #[inline]
    pub unsafe fn from_ptr_unchecked<'a>(b: *const c_char) -> &'a Self {
        Self::from_bytes_unchecked(CStr::from_ptr(b).to_bytes())
    }
}

impl Deref for MemberName {
    type Target = CStr;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[test]
fn t_member_name() {
    MemberName::from_bytes(b"abc13\0").unwrap();
    MemberName::from_bytes(b"abc.13\0").err().unwrap();
    MemberName::from_bytes(b"1234abc\0").err().unwrap();
    MemberName::from_bytes(b"abc").err().unwrap();
    MemberName::from_bytes(b"\0").err().unwrap();
    MemberName::from_bytes(b"a\0").unwrap();
}

// TODO: consider providing a duplicate of this that promises it contains an error
// We need this more general one for writing more direct interfaces into sd-bus, but most user code
// will only encounter an error that is correctly populated by sd-bus itself.
struct RawError {
    inner: ffi::bus::sd_bus_error,
}

pub struct Error {
    raw: RawError,
    name_len: usize,
    message_len: usize,
}

impl Error {
    /// Unsafety:
    ///
    /// - `raw` must be set.
    unsafe fn from_raw(raw: RawError) -> Error {
        let n = CStr::from_ptr(raw.inner.name).to_bytes_with_nul().len();
        let m = if raw.inner.message.is_null() {
            0
        } else {
            CStr::from_ptr(raw.inner.message).to_bytes_with_nul().len()
        };

        Error {
            raw: raw,
            name_len: n,
            message_len: m
        }
    }

    pub fn new(name: &Utf8CStr, message: Option<&Utf8CStr>) -> Error {
        let v = RawError::with(name, message);

        Error {
            raw: v,
            name_len: name.len() + 1,
            message_len: message.map_or(0, |x| x.len() + 1)
        }
    }

    pub fn name(&self) -> &Utf8CStr {
        unsafe { Utf8CStr::from_raw_parts(self.raw.inner.name, self.name_len) }
    }

    pub fn message(&self) -> Option<&Utf8CStr> {
        let p = self.raw.inner.message;
        if p.is_null() {
            None
        } else {
            Some(unsafe { Utf8CStr::from_raw_parts(self.raw.inner.message, self.message_len) })
        }
    }

    fn as_ptr(&self) -> *const ffi::bus::sd_bus_error {
        self.raw.as_ptr()
    }

    unsafe fn move_into(self, dest: *mut ffi::bus::sd_bus_error) {
        let x = ::std::ptr::read(&self.raw.inner);
        forget(self);
        *dest = x;
    }
}

impl ::std::error::Error for Error {
    fn description(&self) -> &str {
        match self.message() {
            Some(m) => m.as_ref(),
            None => self.name().as_ref(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Error")
            .field("name", &self.name())
            .field("message", &self.message())
            .field("need_free", &self.raw.inner.need_free)
            .finish()
    }
}

// TODO: make this display nicer
impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self.message() {
            Some(m) => write!(fmt, "Dbus Error: {}: {}", self.name(), m),
            None => write!(fmt, "Dbus Error: {}", self.name())
        }
    }
}

impl Default for RawError {
    #[inline]
    fn default() -> Self {
        RawError {
            inner: ffi::bus::sd_bus_error {
                name: ptr::null(),
                message: ptr::null(),
                need_free: 0,
            },
        }
    }
}

impl RawError {
    #[inline]
    fn new() -> Self {
        Default::default()
    }

    fn into_result(self) -> Result<()> {
        if self.is_set() {
            Err(unsafe { Error::from_raw(self) })
        } else {
            Ok(())
        }
    }

    fn with(name: &Utf8CStr, message: Option<&Utf8CStr>) -> Self {
        let mut v : Self = Default::default();
        v.set(name, message);
        v
    }

    // XXX: if error is already set, this will not update the error
    // WARNING: using error_set causes strlen() usage even though we already have the lengths
    fn set(&mut self, name: &Utf8CStr, message: Option<&Utf8CStr>) {
        /* return value of sd_bus_error_set is calculated based on name, which we don't care about
         * */
        unsafe {
            ffi::bus::sd_bus_error_set(&mut self.inner,
                                   name.as_ptr(),
                                   message.map_or(ptr::null(), |x| x.as_ptr()));
        }
    }

    #[inline]
    fn is_set(&self) -> bool {
        !self.inner.name.is_null()
    }

    #[inline]
    fn as_mut_ptr(&mut self) -> *mut ffi::bus::sd_bus_error {
        &mut self.inner
    }

    #[inline]
    fn as_ptr(&self) -> *const ffi::bus::sd_bus_error {
        &self.inner
    }

    // XXX: watch out! this method is doing strlen() on every single call to properly construct the
    // reference. Consider caching length info somewhere.
    #[inline]
    pub fn name(&self) -> Option<&InterfaceName> {
        if self.is_set() {
            Some(unsafe{InterfaceName::from_ptr_unchecked(self.inner.name)})
        } else {
            None
        }
    }

    // XXX: watch out! this method is doing strlen() on every single call to properly construct the
    // reference. Consider caching length info somewhere.
    #[inline]
    pub fn message(&self) -> Option<&Utf8CStr> {
        if self.is_set() {
            Some(unsafe{Utf8CStr::from_ptr_unchecked(self.inner.name)})
        } else {
            None
        }
    }

    // TODO: check if the ffi function can fail, and if so in what way
    #[allow(dead_code)]
    #[inline]
    pub fn errno(&self) -> Option<c_int> {
        if self.is_set() {
            Some(unsafe { ffi::bus::sd_bus_error_get_errno(self.as_ptr()) })
        } else {
            None
        }
    }
}

impl Drop for RawError {
    #[inline]
    fn drop(&mut self) {
        unsafe { ffi::bus::sd_bus_error_free(&mut self.inner) };
    }
}

impl Clone for RawError {
    #[inline]
    fn clone(&self) -> RawError {
        let mut e = unsafe { RawError { inner: uninitialized() } };
        unsafe { ffi::bus::sd_bus_error_copy(&mut e.inner, &self.inner) };
        e
    }
}

impl fmt::Debug for RawError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("RawError")
            .field("name", &self.name())
            .field("message", &self.message())
            .field("need_free", &self.inner.need_free)
            .finish()
    }
}

// TODO: make this display nicer
impl fmt::Display for RawError {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("RawError")
            .field("name", &self.name())
            .field("message", &self.message())
            .field("need_free", &self.inner.need_free)
            .finish()
    }
}

#[test]
fn t_raw_error() {
    let name = Utf8CStr::from_bytes(b"name\0").unwrap();
    let message = Utf8CStr::from_bytes(b"error\0").unwrap();
    let _raw = RawError::new().set(name, Some(message));
}

/* XXX: fixme: return code does have meaning! */
extern "C" fn raw_message_handler<F: FnMut(&mut MessageRef) -> Result<()>>(
    msg: *mut ffi::bus::sd_bus_message,
    userdata: *mut c_void,
    ret_error: *mut ffi::bus::sd_bus_error) -> c_int
{
    let m: &mut F = unsafe { transmute(userdata) };
    let e = m(unsafe { MessageRef::from_mut_ptr(msg)});

    match e {
        Err(e) => {
            /* XXX: this relies on ret_error not being allocated data, otherwise we'll leak. */
            unsafe { e.move_into(ret_error) }
            /* If negative, sd_bus_reply_method_errno() is used, which should also work, but this
             * is more direct */
            0
        },
        Ok(_) => {
            /* FIXME: 0 vs positive return codes have different meaning. need to expose/chose
             * properly here */
            0
        }
    }
}

pub struct Bus {
    raw: *mut ffi::bus::sd_bus,
}

impl Bus {
    #[inline]
    pub fn default() -> super::Result<Bus> {
        let mut b = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_default(&mut b));
        Ok(Bus { raw: b })
    }

    #[inline]
    pub fn default_user() -> super::Result<Bus> {
        let mut b = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_default_user(&mut b));
        Ok(Bus { raw: b })
    }

    #[inline]
    pub fn default_system() -> super::Result<Bus> {
        let mut b = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_default_system(&mut b));
        Ok(Bus { raw: b })
    }

    #[inline]
    unsafe fn from_ptr(r: *mut ffi::bus::sd_bus) -> Bus {
        Bus { raw: ffi::bus::sd_bus_ref(r) }
    }

    // unsafe fn take_ptr(r: *mut ffi::bus::sd_bus) -> Bus {
    // Bus { raw: r }
    // }
    //

    #[inline]
    fn as_ptr(&self) -> *const ffi::bus::sd_bus {
        self.raw
    }

    #[inline]
    fn as_mut_ptr(&mut self) -> *mut ffi::bus::sd_bus {
        self.raw
    }
}

impl Borrow<BusRef> for Bus {
    #[inline]
    fn borrow(&self) -> &BusRef {
        unsafe { BusRef::from_ptr(self.as_ptr()) }
    }
}

impl BorrowMut<BusRef> for Bus {
    #[inline]
    fn borrow_mut(&mut self) -> &mut BusRef {
        unsafe { BusRef::from_mut_ptr(self.as_mut_ptr()) }
    }
}

impl Deref for Bus {
    type Target = BusRef;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl DerefMut for Bus {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.borrow_mut()
    }
}

impl Drop for Bus {
    #[inline]
    fn drop(&mut self) {
        unsafe { ffi::bus::sd_bus_unref(self.raw) };
    }
}

impl Clone for Bus {
    #[inline]
    fn clone(&self) -> Bus {
        Bus { raw: unsafe { ffi::bus::sd_bus_ref(self.raw) } }
    }
}

pub struct BusRef {
    _inner: ffi::bus::sd_bus,
}

impl fmt::Debug for BusRef {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("BusRef").finish()
    }
}

impl ToOwned for BusRef {
    type Owned = Bus;
    #[inline]
    fn to_owned(&self) -> Self::Owned {
        unsafe { Bus::from_ptr(self.as_ptr()) }
    }
}

impl BusRef {
    #[inline]
    unsafe fn from_ptr<'a>(r: *const ffi::bus::sd_bus) -> &'a BusRef {
        transmute(r)
    }

    #[inline]
    unsafe fn from_mut_ptr<'a>(r: *mut ffi::bus::sd_bus) -> &'a mut BusRef {
        transmute(r)
    }

    #[inline]
    pub fn to_owned(&self) -> Bus {
        unsafe { Bus::from_ptr(self.as_ptr()) }
    }

    #[inline]
    fn as_ptr(&self) -> *mut ffi::bus::sd_bus {
        unsafe { transmute(self) }
    }

    #[inline]
    pub fn events(&self) -> super::Result<c_int> {
        Ok(sd_try!(ffi::bus::sd_bus_get_events(self.as_ptr())))
    }

    #[inline]
    pub fn timeout(&self) -> super::Result<u64> {
        let mut b = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_get_timeout(self.as_ptr(), &mut b));
        Ok(b)
    }

    #[inline]
    pub fn fd(&self) -> super::Result<c_int> {
        Ok(sd_try!(ffi::bus::sd_bus_get_fd(self.as_ptr())))
    }

    #[inline]
    pub fn unique_name(&self) -> super::Result<&BusName> {
        let mut e = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_get_unique_name(self.as_ptr(), &mut e));
        Ok(unsafe { BusName::from_ptr_unchecked(e) })
    }

    #[inline]
    pub fn new_signal(&mut self,
                      path: &ObjectPath,
                      interface: &InterfaceName,
                      member: &MemberName)
                      -> super::Result<Message> {
        let mut m = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_message_new_signal(self.as_ptr(),
                                                    &mut m,
                                                    path.as_ptr() as *const _,
                                                    interface.as_ptr() as *const _,
                                                    member.as_ptr() as *const _));
        Ok(unsafe { Message::take_ptr(m) })
    }

    #[inline]
    pub fn new_method_call(&mut self,
                           dest: &BusName,
                           path: &ObjectPath,
                           interface: &InterfaceName,
                           member: &MemberName)
                           -> super::Result<Message> {
        let mut m = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_message_new_method_call(self.as_ptr(),
                                                         &mut m,
                                                         &*dest as *const _ as *const _,
                                                         &*path as *const _ as *const _,
                                                         &*interface as *const _ as *const _,
                                                         &*member as *const _ as *const _));
        Ok(unsafe { Message::take_ptr(m) })
    }

    // new_method_errno

    // TODO: consider using a guard object for name handling
    /// This blocks. To get async behavior, use 'call_async' directly.
    #[inline]
    pub fn request_name(&self, name: &BusName, flags: u64) -> super::Result<()> {
        sd_try!(ffi::bus::sd_bus_request_name(self.as_ptr(),
                                              &*name as *const _ as *const _,
                                              flags));
        Ok(())
    }

    /// This blocks. To get async behavior, use 'call_async' directly.
    #[inline]
    pub fn release_name(&self, name: &BusName) -> super::Result<()> {
        sd_try!(ffi::bus::sd_bus_release_name(self.as_ptr(), &*name as *const _ as *const _));
        Ok(())
    }

    // XXX: alternates for (userdata: T):
    //  - userdata: T, and automatically box as needed. Allows a useful external control.
    //  - userdata: Box<T>, allows users to supply a box directly if they already have one
    //  - userdata: &mut T, allows users to manage lifetime of passed in values direcly
    //  - userdata: SizeMatches<*const _>, allows users to use things without a pointer indirection
    //    (such as integer types). Not clear this is possible in rust today (1.9).
    //  - cb: &FnMut
    //  - cb: &CustomTrait
    #[inline]
    pub fn add_object<F: FnMut(&mut MessageRef) -> Result<()>>(&self,
                                                                      path: &ObjectPath,
                                                                      cb: &mut F)
                                                                      -> super::Result<()> {
        let f: extern "C" fn(*mut ffi::bus::sd_bus_message,
                             *mut c_void,
                             *mut ffi::bus::sd_bus_error)
                             -> c_int = raw_message_handler::<F>;
        sd_try!(ffi::bus::sd_bus_add_object(self.as_ptr(),
                                            ptr::null_mut(),
                                            &*path as *const _ as *const _,
                                            Some(f),
                                            cb as *mut _ as *mut _));
        Ok(())
    }

    #[inline]
    pub fn add_object_manager(&self, path: &ObjectPath) -> super::Result<()> {
        sd_try!(ffi::bus::sd_bus_add_object_manager(self.as_ptr(),
                                                    ptr::null_mut(),
                                                    &*path as *const _ as *const _));
        Ok(())
    }

    // pub fn add_object_vtable<T: Any + 'static>(&self,
    //                                           path: ObjectPath,
    //                                           interface: InterfaceName,
    //                                           vtable: Vtable<T>,
    //                                           userdata: T)
    //                                           -> super::Result<()> {
    //    let u = Box::into_raw(Box::new(userdata));
    //    sd_try!(ffi::bus::sd_bus_add_object_vtable(self.raw,
    //                                               ptr::null_mut(),
    //                                               path.as_ptr() as *const _,
    //                                               interface.as_ptr() as *const _,
    //                                               vtable.as_ptr(),
    //                                               Box::into_raw(Box::new(T))));
    //    Ok(())
    // }


    // emit_signal
    // emit_properties_changed
    // emit_object_added
    // emit_object_removed
    // emit_interfaces_added
    // emit_interfaces_removed

    // track
}

impl AsRawFd for BusRef {
    #[inline]
    fn as_raw_fd(&self) -> c_int {
        self.fd().unwrap()
    }
}

/*
extern "C" fn raw_track_handler<F: FnMut(Track) -> c_int>(
    track: *mut ffi::bus::sd_bus_track, userdata: *mut c_void) -> c_int
{
    let m : &mut F = unsafe { transmute(userdata) };
    m(Track::from_ptr(track))
}

pub struct Track {
    raw: *mut ffi::bus::sd_bus_track
}

impl Track {
    unsafe fn from_ptr(track: *mut ff::bus::sd_bus_track) {
        Track { raw: unsafe { ffi::bus::sd_bus_tracK_ref(tracK) } }
    }

    fn new<F: FnMut(Track)>(bus: &mut Bus, handler: F) -> super::Result<Track> {
    }
}
*/

/*
 * TODO: determine if the lifetime of a message is tied to the lifetime of the bus used to create
 * it
 */

/// A message to be sent or that was recieved over dbus
///
/// This is reference counted, clone does not copy the type
pub struct Message {
    raw: *mut ffi::bus::sd_bus_message,
}

/// A reference to a `Message`
pub struct MessageRef {
    _inner: ffi::bus::sd_bus_message
}

/// An iterator over the elements of a `Message`, use this to read data out of a message.
///
/// Note: we're using a concrete type here instead of a reference to allow us to handle lifetimes
/// properly.
pub struct MessageIter<'a> {
    raw: *mut ffi::bus::sd_bus_message,
    life: PhantomData<&'a MessageRef>
}

impl Message {
    /**
     * Construct a Message, taking over an already existing reference count on the provided pointer
     *
     * To construct a Message from an un-owned pointer, use MessageRef::from_ptr(p).to_owned()
     */
    #[inline]
    unsafe fn take_ptr(p: *mut ffi::bus::sd_bus_message) -> Message {
        Message { raw: p }
    }

    // fn into_ptr(mut self) -> *mut ffi::bus::sd_bus_message {
    // let r = self.as_mut_ptr();
    // forget(self);
    // r
    // }
    //
}

impl Drop for Message {
    #[inline]
    fn drop(&mut self) {
        unsafe { ffi::bus::sd_bus_message_unref(self.raw) };
    }
}

impl Clone for Message {
    #[inline]
    fn clone(&self) -> Message {
        Message { raw: unsafe { ffi::bus::sd_bus_message_ref(self.raw) } }
    }
}

impl Deref for Message {
    type Target = MessageRef;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { MessageRef::from_ptr(self.raw) }
    }
}

impl DerefMut for Message {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { MessageRef::from_mut_ptr(self.raw) }
    }
}

impl Borrow<MessageRef> for Message {
    #[inline]
    fn borrow(&self) -> &MessageRef {
        self.deref()
    }
}

impl BorrowMut<MessageRef> for Message {
    #[inline]
    fn borrow_mut(&mut self) -> &mut MessageRef {
        self.deref_mut()
    }
}

// Warning: going from a &MessageRef to a Message bypasses some of the borrow checking (allows us
// to have multiple mutable references to the same data). This issue is all over the place in
// sd-bus.
//
impl ToOwned for MessageRef {
    type Owned = Message;
    #[inline]
    fn to_owned(&self) -> Self::Owned {
        Message { raw: unsafe { ffi::bus::sd_bus_message_ref(self.as_ptr() as *mut _) } }
    }
}

impl MessageRef {
    #[inline]
    unsafe fn from_ptr<'a>(p: *const ffi::bus::sd_bus_message) -> &'a MessageRef {
        transmute(p)
    }

    #[inline]
    unsafe fn from_mut_ptr<'a>(p: *mut ffi::bus::sd_bus_message) -> &'a mut MessageRef {
        transmute(p)
    }

    #[inline]
    fn as_ptr(&self) -> *const ffi::bus::sd_bus_message {
        unsafe { transmute(self) }
    }

    #[inline]
    fn as_mut_ptr(&mut self) -> *mut ffi::bus::sd_bus_message {
        unsafe { transmute(self) }
    }

    /* FIXME: unclear that the mut handling is correct in all of this code (not just this function)
     * */
    #[inline]
    pub fn bus(&self) -> &BusRef {
        unsafe { BusRef::from_mut_ptr(ffi::bus::sd_bus_message_get_bus(self.as_ptr() as *mut _)) }
    }

    /// Set the message destination, the name of the bus client we want to send this message to.
    ///
    /// XXX: describe broadcast
    ///
    /// Fails if the message is sealed
    #[inline]
    pub fn set_destination(&mut self, dest: &BusName) -> super::Result<()> {
        sd_try!(ffi::bus::sd_bus_message_set_destination(self.as_mut_ptr(),
                    &*dest as *const _ as *const _));
        Ok(())
    }

    /// Set to true to allow the bus to launch an owner for the destination name.
    ///
    /// Set to false to prevent the bus from launching an owner for the destination name.
    ///
    /// Fails if the message is sealed
    ///
    /// ---
    ///
    /// This controls the NO_AUTO_START dbus header flag.
    ///
    /// The
    /// [specification](https://dbus.freedesktop.org/doc/dbus-specification.html#message-bus-starting-services)
    /// covers some details about the auto start mechanism, but not all of it is specified.
    #[inline]
    pub fn set_auto_start(&mut self, yes: bool) -> super::Result<()> {
        sd_try!(ffi::bus::sd_bus_message_set_auto_start(self.as_mut_ptr(), yes as c_int));
        Ok(())
    }

    // # properties
    // type
    // cookie
    // reply_cookie
    // priority
    // expect_reply
    // auto_start
    // allow_interactive_authorization
    // signature
    // path
    // interface
    // member
    // destination
    // sender
    // error
    // errno
    // monotonic_usec
    // realtime_usec
    // seqnum

    // is_signal
    // is_method_call
    // is_method_error
    // is_empty
    // has_signature

    /*
     * send (and it's wrappers below) keeps a reference to the Message, and really wants to own it
     * (it seals the message against further modification). Ideally we'd make it clearer in the API
     * that this is the case to prevent folks from accidentally trying to modify a message after
     * sending it
     */

    /// Send expecting a reply. Returns the reply cookie.
    ///
    /// Seals `self`.
    #[inline]
    pub fn send(&mut self) -> super::Result<u64> {
        // self.bus().send(self)
        let mut m = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_send(ptr::null_mut(), self.as_mut_ptr(), &mut m));
        Ok(m)
    }

    /// Send without expecting any reply
    /// Seals `self`.
    #[inline]
    pub fn send_no_reply(&mut self) -> super::Result<()> {
        // self.bus().send_no_reply(self)
        sd_try!(ffi::bus::sd_bus_send(ptr::null_mut(), self.as_mut_ptr(), ptr::null_mut()));
        Ok(())
    }

    /// Send this message to a destination.
    ///
    /// Internally, this is the same as `.set_destination()` + `.send()`
    /// Seals `self`.
    #[inline]
    pub fn send_to(&mut self, dest: &BusName) -> super::Result<u64> {
        // self.bus().send_to(self, dest)
        let mut c = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_send_to(ptr::null_mut(),
                                         self.as_mut_ptr(),
                                         &*dest as *const _ as *const _,
                                         &mut c));
        Ok(c)
    }

    /// Same as `self.send_to()`, but don't expect a reply.
    /// Seals `self`.
    #[inline]
    pub fn send_to_no_reply(&mut self, dest: &BusName) -> super::Result<()> {
        // self.bus().send_to_no_reply(self, dest)
        sd_try!(ffi::bus::sd_bus_send_to(ptr::null_mut(),
                                         self.as_mut_ptr(),
                                         &*dest as *const _ as *const _,
                                         ptr::null_mut()));
        Ok(())
    }

    /// Use this message to call a dbus method. Blocks until a reply is recieved or `usec`
    /// microseconds elapse (ie: this times out)
    ///
    /// XXX: document blocking forever
    /// Seals `self`.
    #[inline]
    pub fn call(&mut self, usec: u64) -> Result<Message> {
        let mut r = unsafe { uninitialized() };
        let mut e = RawError::new();
        unsafe {
            ffi::bus::sd_bus_call(ptr::null_mut(),
                    self.as_mut_ptr(),
                    usec,
                    e.as_mut_ptr(),
                    &mut r);
        }
        e.into_result().map(|_| unsafe { Message::take_ptr(r)})
    }

    /// Use this message to call a dbus method. Returns immediately and will call the callback when
    /// a reply is recieved.
    ///
    /// XXX: document how timeout affects this
    /// Seals `self`.
    // XXX: we may need to move this, unclear we have the right lifetime here (we're being too
    // strict)
    #[inline]
    pub fn call_async<F: FnMut(&mut MessageRef) -> Result<()>>(&mut self,
                                                                      callback: &mut F,
                                                                      usec: u64)
                                                                      -> super::Result<()> {
        let f: extern "C" fn(*mut ffi::bus::sd_bus_message,
                             *mut c_void,
                             *mut ffi::bus::sd_bus_error)
                             -> c_int = raw_message_handler::<F>;
        sd_try!(ffi::bus::sd_bus_call_async(ptr::null_mut(),
                                            ptr::null_mut(),
                                            self.as_mut_ptr(),
                                            Some(f),
                                            callback as *mut _ as *mut _,
                                            usec));
        Ok(())
    }

    #[inline]
    pub fn new_method_error(&mut self, error: &Error) -> super::Result<Message> {
        let mut m = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_message_new_method_error(self.as_mut_ptr(), &mut m, error.as_ptr()));
        Ok(unsafe { Message::take_ptr(m) })
    }

    #[inline]
    pub fn new_method_return(&mut self) -> super::Result<Message> {
        let mut m = unsafe { uninitialized() };
        sd_try!(ffi::bus::sd_bus_message_new_method_return(self.as_mut_ptr(), &mut m));
        Ok(unsafe { Message::take_ptr(m) })
    }

    /// Raw access to append data to this message
    /// Will fail if the message is sealed
    // XXX: unclear if this should operate directly on the message or be split out to the iterator
    // mechanism
    #[inline]
    pub unsafe fn append_basic_raw(&mut self, dbus_type: u8, v: *const c_void) -> ::Result<()> {
        try!(::ffi_result(ffi::bus::sd_bus_message_append_basic(self.as_mut_ptr(), dbus_type as c_char, v)));
        Ok(())
    }

    /// Append a value to the message
    #[inline]
    pub fn append<V: types::ToSdBusMessage>(&mut self, v: V) -> ::Result<()> {
        v.to_message(self)
    }

    /// Get an iterator over the message. This iterator really exists with in the `Message` itself,
    /// so we can only hand out one at a time.
    ///
    /// Ideally, handing this iterator out wouldn't prevent the use of other non-iterator
    /// accessors, but right now it does (unless you bypass `borrowck` using `unsafe{}`)
    ///
    /// Requires that message is sealed.
    #[inline]
    pub fn iter<'a>(&'a mut self) -> ::Result<MessageIter<'a>> {
        /* probe the `Message` to check if we can iterate on it */
        sd_try!(ffi::bus::sd_bus_message_peek_type(self.as_mut_ptr(), ptr::null_mut(), ptr::null_mut()));
        Ok(MessageIter { raw: self.as_mut_ptr(), life: PhantomData })
    }

}

impl<'a> MessageIter<'a> {
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut ffi::bus::sd_bus_message {
        self.raw
    }

    /*
     * XXX: 'T' may reference the parent `Message`, and should be tied to the lifetime of the
     * `MessageIter` (to ensure they don't change out from underneath us) but shouldn't be tied to
     * the lifetime of the &mut self of this call
     */
    /// Read an element from the message and advance the internal cursor
    /// References returned by this function are valid until the iterator itself is dropped (just
    /// to garuntee they don't change).
    ///
    /// XXX: really, they are valid until the message is un-sealed: reading from the message can
    /// only occur while the message is sealed. Unclear if we can track lifetimes against message
    /// sealing.
    ///
    /// Unsafety:
    ///
    ///  - `dbus_type` when given to `sd_bus_message_read_basic()` must result in something that
    ///    can be reinterpreted as `R`.
    #[inline]
    pub unsafe fn read_basic_raw<R, T, F: FnOnce(R) -> T>(&mut self, dbus_type: u8, cons: F)
            -> ::Result<Option<T>>
        where T: 'a
    {
        let mut v: R = uninitialized();
        match ::ffi_result(ffi::bus::sd_bus_message_read_basic(self.as_mut_ptr(), dbus_type as c_char, &mut v as *mut _ as *mut _)) {
            Ok(1) => Ok(Some(cons(v))),
            Ok(_) => {
                forget(v);
                Ok(None)
            },
            Err(e) => {
                forget(v);
                Err(e)
            }
        }
    }

    /// This needs to be `&mut` as the `&str` will be invalid after either of:
    ///  - self is dropped
    ///  - sd_bus_message_peek_type is called a second time
    ///
    /// Using &mut allows us to prevent #2.
    ///
    /// FIXME/WARNING: Message might have been cloned, in which case we can't rely on the lifetime of
    /// &str! As `Message` isn't `Send` or `Sync`, we can garuntee we're not racing with someone
    /// else to free it though. Probably need to allocate space for it here rather than return a
    /// ref.
    // &str lasts until next call of sd_bus_message_peek_type
    // XXX: confirm that lifetimes here match that!
    #[inline]
    pub fn peek_type(&mut self) -> ::Result<(c_char, &str)>
    {
        let mut t: c_char = unsafe { uninitialized() };
        let mut cont: *const c_char = unsafe { uninitialized() };
        try!(::ffi_result(unsafe { ffi::bus::sd_bus_message_peek_type(self.as_mut_ptr(), &mut t, &mut cont) }));

        let s = if cont.is_null() {
            /* XXX: we may need to adjust here and return an option, but it isn't yet clear if
             * there will be confusion between NULL and "" here */
            ""
        } else {
            unsafe {str::from_utf8_unchecked(CStr::from_ptr(cont).to_bytes())}
        };
        Ok((t, s))
    }

    // XXX: handle containers

    pub fn next<V: types::FromSdBusMessage<'a>>(&'a mut self) -> ::Result<Option<V>>
    {
        V::from_message(self)
    }
}

/*
struct Vtable;
struct VtableBuilder<T> {
    Vec<ffi::bus::sd_bus_vtable>,
}

type PropertyGet<T> = fn(Bus, ObjectPath, InterfaceName, MessageRef, &mut T, &mut Error) -> c_int;
type PropertySet<T> = fn(Bus, ObjectPath, InterfaceName, MessageRef, &mut T, &mut Error) -> c_int;


impl VtableBuilder {
    fn method(mut self, member: &str, signature: &str, result: &str, handler: MessageHandler) {
        /* verify */
        /* track */
    }

    fn property(mut self, member: &str, signature: &str, get: PropertyGet) {
    }

    fn property_writable(mut self, member: &str, signature: &str, get: PropertyGet, set: PropertySet) {
    }

    fn signal(mut self, member: &str, signature: &str) {
    }

    fn create(mut self) -> Vtable {
    }
}
*/
