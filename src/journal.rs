use libc::{c_char, c_int, size_t};
use std::{io, ptr};
use std::ffi::CString;
use std::io::ErrorKind::InvalidData;
use ffi::id128::sd_id128_t;
use ffi::journal as ffi;
use id128::Id128;
use super::Result;
use mbox::MString;

pub struct Journal {
    j: *mut ffi::sd_journal,
    sz: size_t,
    data: *mut u8,
}

/// Represents the set of journal files to read.
pub enum JournalFiles {
    /// The system-wide journal.
    System,
    /// The current user's journal.
    CurrentUser,
    /// Both the system-wide journal and the current user's journal.
    All,
}

/// Seeking position in journal.
pub enum JournalSeek {
    Head,
    Current,
    Tail,
    ClockMonotonic {
        boot_id: Id128,
        usec: u64,
    },
    ClockRealtime {
        usec: u64,
    },
    Cursor {
        cursor: String,
    },
}

impl Journal {
    /// Open the systemd journal for reading.
    ///
    /// Params:
    ///
    /// * files: the set of journal files to read. If the calling process
    ///   doesn't have permission to read the system journal, a call to
    ///   `Journal::open` with `System` or `All` will succeed, but system
    ///   journal entries won't be included. This behavior is due to systemd.
    /// * runtime_only: if true, include only journal entries from the current
    ///   boot. If false, include all entries.
    /// * local_only: if true, include only journal entries originating from
    ///   localhost. If false, include all entries.
    pub fn open(files: JournalFiles, runtime_only: bool, local_only: bool) -> Result<Journal> {
        let mut flags: c_int = 0;
        if runtime_only {
            flags |= ffi::SD_JOURNAL_RUNTIME_ONLY;
        }
        if local_only {
            flags |= ffi::SD_JOURNAL_LOCAL_ONLY;
        }
        flags |= match files {
            JournalFiles::System => ffi::SD_JOURNAL_SYSTEM,
            JournalFiles::CurrentUser => ffi::SD_JOURNAL_CURRENT_USER,
            JournalFiles::All => 0,
        };

        let mut journal = Journal { j: ptr::null_mut() , sz: 0, data: ptr::null_mut()};
        sd_try!(ffi::sd_journal_open(&mut journal.j, flags));
        Ok(journal)
    }

    /// Get and parse the currently journal record from the journal
    pub fn get_next_field(&mut self) -> Result<Option<(&str, &str)>> {


        if sd_try!(ffi::sd_journal_enumerate_data(self.j, &self.data, &mut self.sz)) > 0 {
            unsafe {
                let b = ::std::slice::from_raw_parts_mut(self.data, self.sz as usize);
                let field = ::std::str::from_utf8_unchecked(b);
                let mut name_value = field.splitn(2, '=');
                let name = name_value.next().unwrap();
                let value = name_value.next().unwrap();
                Ok(Some((name, value)))
            }
            
        }else{
            Ok(None)
        }

        
    }

    pub fn previous_record(&mut self) ->Result<Option<i32>> {
        let r = sd_try!(ffi::sd_journal_previous(self.j));
        unsafe { ffi::sd_journal_restart_data(self.j) }
        self.sz = 0;
        self.data = ptr::null_mut();
        if r == 0{
            Ok(None)
        }else{
            Ok(Some(r))
        }
    }

    /// Seek to a specific position in journal. On success, returns a cursor
    /// to the current entry.
    pub fn seek(&mut self, seek: JournalSeek) -> Result<String> {
        match seek {
            JournalSeek::Head => sd_try!(ffi::sd_journal_seek_head(self.j)),
            JournalSeek::Current => 0,
            JournalSeek::Tail => sd_try!(ffi::sd_journal_seek_tail(self.j)),
            JournalSeek::ClockMonotonic { boot_id, usec } => {
                sd_try!(ffi::sd_journal_seek_monotonic_usec(self.j,
                                                            sd_id128_t {
                                                                bytes: *boot_id.as_bytes(),
                                                            },
                                                            usec))
            }
            JournalSeek::ClockRealtime { usec } => {
                sd_try!(ffi::sd_journal_seek_realtime_usec(self.j, usec))
            }
            JournalSeek::Cursor { cursor } => {
                let c = try!(CString::new(cursor));
                sd_try!(ffi::sd_journal_seek_cursor(self.j, c.as_ptr()))
            }
        };
        let c: *mut c_char = ptr::null_mut();
        if unsafe { ffi::sd_journal_get_cursor(self.j, &c) != 0 } {
            // Cursor may need to be re-aligned on a real entry first.
            sd_try!(ffi::sd_journal_next(self.j));
            sd_try!(ffi::sd_journal_get_cursor(self.j, &c));
        }
        let cs = unsafe { MString::from_raw(c) };
        let cs = try!(cs.or(Err(io::Error::new(InvalidData, "invalid cursor"))));
        Ok(cs.to_string())
    }

    /// Returns the cursor of current journal entry
    pub fn cursor(&self) -> Result<String> {
        let mut c_cursor: *mut c_char = ptr::null_mut();

        sd_try!(ffi::sd_journal_get_cursor(self.j, &mut c_cursor));

        let cursor = unsafe { MString::from_raw(c_cursor) };
        let cursor = try!(cursor.or(Err(io::Error::new(InvalidData, "invalid cursor"))));
        Ok(cursor.to_string())
    }

    
}
