extern crate libc;
extern crate libfaster_sys as ffi;

mod builder;
mod faster_error;
pub mod status;
mod util;

pub use crate::builder::{FasterKvConfig, HlogCompactionConfig, ReadCacheConfig};
pub use crate::faster_error::FasterError;
use crate::util::*;
use linux_futex::{Futex, Private};

use std::ffi::CStr;
use std::ffi::CString;
use std::fs;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(not(target_os = "linux"))]
compile_error!("faster-rs raw-bytes API requires Linux futex support");

#[unsafe(no_mangle)]
/// # Safety
/// Caller must pass a pointer previously allocated by `Box<[u8]>` with the given length.
pub unsafe extern "C" fn deallocate_vec(vec: *mut u8, length: u64) {
    unsafe {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            vec,
            length as usize,
        )));
    }
}

#[unsafe(no_mangle)]
/// # Safety
/// Caller must treat the returned pointer as a Rust-owned allocation and free it with `deallocate_vec`.
pub unsafe extern "C" fn faster_alloc_vec(length: u64) -> *mut u8 {
    let bytes = vec![0u8; length as usize].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8
}

#[unsafe(no_mangle)]
/// # Safety
/// Caller must initialize all fields before use and free it with `Box::from_raw`.
pub unsafe extern "C" fn faster_alloc_checkpoint_result() -> *mut ffi::faster_checkpoint_result {
    let result =
        unsafe { Box::new(MaybeUninit::<ffi::faster_checkpoint_result>::zeroed().assume_init()) };
    Box::into_raw(result)
}

#[unsafe(no_mangle)]
/// # Safety
/// Caller must initialize all fields before use and free it with `Box::from_raw`.
pub unsafe extern "C" fn faster_alloc_recover_result() -> *mut ffi::faster_recover_result {
    let result =
        unsafe { Box::new(MaybeUninit::<ffi::faster_recover_result>::zeroed().assume_init()) };
    Box::into_raw(result)
}

#[inline(always)]
/// # Safety
/// `target` must be a valid `ReadSlot` pointer created from a `Box<ReadSlot>`.
pub unsafe extern "C" fn read_callback(
    target: *mut libc::c_void,
    value: *const u8,
    length: u64,
    status: u32,
) {
    if target.is_null() {
        return;
    }
    let slot = unsafe { &*(target as *const ReadSlot) };
    if status == status::OK.into() {
        if length > 0 && value.is_null() {
            slot.status.store(status::ABORTED.into(), Ordering::Release);
        } else if length == 0 {
            let bytes: Box<[u8]> = Box::new([]);
            let ptr = Box::into_raw(bytes) as *mut u8;
            slot.buffer.store(ptr, Ordering::Release);
            slot.len.store(0, Ordering::Release);
            slot.status.store(status, Ordering::Release);
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(value, length as usize) }
                .to_vec()
                .into_boxed_slice();
            let ptr = Box::into_raw(bytes) as *mut u8;
            slot.buffer.store(ptr, Ordering::Release);
            slot.len.store(length as usize, Ordering::Release);
            slot.status.store(status, Ordering::Release);
        }
    } else {
        slot.status.store(status, Ordering::Release);
    }
    slot.futex.value.store(1, Ordering::Release);
    let _ = slot.futex.wake(1);
    let state = slot.state.fetch_or(STATE_DONE, Ordering::AcqRel);
    if state & STATE_DROPPED != 0 {
        unsafe {
            free_slot(target as *mut ReadSlot);
        }
    }
}

pub struct ReadWaiter {
    slot: NonNull<ReadSlot>,
}

#[derive(Debug)]
pub enum ReadError {
    NotFound,
    Empty,
}

impl ReadWaiter {
    pub fn recv(self) -> Result<Vec<u8>, ReadError> {
        let slot = unsafe { self.slot.as_ref() };
        slot.wait_ready();
        let status = slot.status.load(Ordering::Acquire);
        if status != status::OK.into() {
            return Err(ReadError::NotFound);
        }
        let len = slot.len.load(Ordering::Acquire);
        let ptr = slot.buffer.load(Ordering::Acquire);
        if ptr.is_null() {
            return Err(ReadError::Empty);
        }
        let boxed = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
        slot.buffer.store(std::ptr::null_mut(), Ordering::Release);
        slot.len.store(0, Ordering::Release);
        Ok(boxed.into_vec())
    }
}

impl Drop for ReadWaiter {
    fn drop(&mut self) {
        let slot = unsafe { self.slot.as_ref() };
        let state = slot.state.fetch_or(STATE_DROPPED, Ordering::AcqRel);
        if state & STATE_DONE != 0 {
            unsafe {
                free_slot(self.slot.as_ptr());
            }
        }
    }
}

struct ReadSlot {
    futex: Futex<Private>,
    // Bitflags: DONE and DROPPED.
    state: AtomicU32,
    status: AtomicU32,
    buffer: AtomicPtr<u8>,
    len: AtomicUsize,
}

impl ReadSlot {
    fn new() -> Self {
        Self {
            futex: Futex::new(0),
            state: AtomicU32::new(0),
            status: AtomicU32::new(status::NOT_FOUND.into()),
            buffer: AtomicPtr::new(std::ptr::null_mut()),
            len: AtomicUsize::new(0),
        }
    }

    fn wait_ready(&self) {
        while self.futex.value.load(Ordering::Acquire) == 0 {
            let _ = self.futex.wait(0);
        }
    }
}

const STATE_DONE: u32 = 1;
const STATE_DROPPED: u32 = 2;

unsafe fn free_slot(slot: *mut ReadSlot) {
    let slot_ref = unsafe { &*slot };
    let ptr = slot_ref.buffer.load(Ordering::Acquire);
    let len = slot_ref.len.load(Ordering::Acquire);
    if !ptr.is_null() {
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
        }
    }
    unsafe {
        drop(Box::from_raw(slot));
    }
}

pub struct FasterKv {
    faster_t: *mut ffi::faster_t,
    storage_dir: Option<String>,
}

impl FasterKv {
    pub fn upsert<K, V>(&self, key: &K, value: &V, monotonic_serial_number: u64) -> u8
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let encoded_key = key.as_ref().to_vec().into_boxed_slice();
        let encoded_key_length = encoded_key.len();
        let encoded_key_ptr = Box::into_raw(encoded_key) as *mut u8;
        let encoded_value = value.as_ref().to_vec().into_boxed_slice();
        let encoded_value_length = encoded_value.len();
        let encoded_value_ptr = Box::into_raw(encoded_value) as *mut u8;
        unsafe {
            ffi::faster_upsert(
                self.faster_t,
                encoded_key_ptr,
                encoded_key_length as u64,
                encoded_value_ptr,
                encoded_value_length as u64,
                monotonic_serial_number,
            )
        }
    }

    pub fn read<K>(&self, key: &K, monotonic_serial_number: u64) -> (u8, ReadWaiter)
    where
        K: AsRef<[u8]>,
    {
        let encoded_key = key.as_ref().to_vec().into_boxed_slice();
        let encoded_key_length = encoded_key.len();
        let encoded_key_ptr = Box::into_raw(encoded_key) as *mut u8;
        let slot = Box::new(ReadSlot::new());
        let target = Box::into_raw(slot) as *mut libc::c_void;
        let status = unsafe {
            ffi::faster_read(
                self.faster_t,
                encoded_key_ptr,
                encoded_key_length as u64,
                monotonic_serial_number,
                Some(read_callback),
                target,
            )
        };
        if status != status::OK && status != status::PENDING {
            let slot_ref = unsafe { &*(target as *const ReadSlot) };
            slot_ref.status.store(status.into(), Ordering::Release);
            slot_ref.futex.value.store(1, Ordering::Release);
            let _ = slot_ref.futex.wake(1);
            let state = slot_ref.state.fetch_or(STATE_DONE, Ordering::AcqRel);
            if state & STATE_DROPPED != 0 {
                unsafe {
                    free_slot(target as *mut ReadSlot);
                }
            }
        }
        let slot = unsafe { NonNull::new_unchecked(target as *mut ReadSlot) };
        (status, ReadWaiter { slot })
    }

    /// Deletes a previously inserted key.
    ///
    /// Returns [NOT_FOUND](status/constant.NOT_FOUND.html) for un-inserted keys.
    ///
    /// # Example
    /// ```
    /// use faster_rs::{FasterKv, status};
    /// let store = FasterKv::default();
    ///
    /// let key = 1u64.to_le_bytes();
    /// let value = 42u64.to_le_bytes();
    ///
    /// // Insert key-value
    /// store.upsert(&key, &value, 1);
    ///
    /// // Read key-value
    /// let (res, recv) = store.read(&key, 1);
    /// assert_eq!(status::OK, res);
    /// assert_eq!(value.as_slice(), recv.recv().unwrap().as_slice());
    ///
    /// // Delete key-value
    /// store.delete(&key, 1);
    ///
    /// // Re-read key-value and confirm deleted
    /// let (res, recv) = store.read(&key, 1);
    /// assert_eq!(status::NOT_FOUND, res);
    /// assert!(recv.recv().is_err());
    /// ```
    pub fn delete<K>(&self, key: &K, monotonic_serial_number: u64) -> u8
    where
        K: AsRef<[u8]>,
    {
        let encoded_key = key.as_ref().to_vec().into_boxed_slice();
        let encoded_key_length = encoded_key.len();
        let encoded_key_ptr = Box::into_raw(encoded_key) as *mut u8;
        unsafe {
            ffi::faster_delete(
                self.faster_t,
                encoded_key_ptr,
                encoded_key_length as u64,
                monotonic_serial_number,
            )
        }
    }

    pub fn size(&self) -> u64 {
        unsafe { ffi::faster_size(self.faster_t) }
    }

    pub fn num_active_sessions(&self) -> u32 {
        unsafe { ffi::faster_num_active_sessions(self.faster_t) }
    }

    pub fn auto_compaction_scheduled(&self) -> bool {
        unsafe { ffi::faster_auto_compaction_scheduled(self.faster_t) }
    }

    pub fn hlog_max_size_reached(&self) -> bool {
        unsafe { ffi::faster_hlog_max_size_reached(self.faster_t) }
    }

    pub fn hlog_begin_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_begin_address(self.faster_t) }
    }

    pub fn hlog_tail_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_tail_address(self.faster_t) }
    }

    pub fn hlog_head_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_head_address(self.faster_t) }
    }

    pub fn hlog_safe_head_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_safe_head_address(self.faster_t) }
    }

    pub fn hlog_read_only_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_read_only_address(self.faster_t) }
    }

    pub fn hlog_safe_read_only_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_safe_read_only_address(self.faster_t) }
    }

    pub fn hlog_flushed_until_address(&self) -> u64 {
        unsafe { ffi::faster_hlog_flushed_until_address(self.faster_t) }
    }

    pub fn checkpoint(&self) -> Result<CheckPoint, FasterError<'_>> {
        if self.storage_dir.is_none() {
            return Err(FasterError::InvalidType);
        }

        let result = unsafe { ffi::faster_checkpoint(self.faster_t) };
        match result.is_null() {
            true => Err(FasterError::CheckpointError),
            false => {
                let boxed = unsafe { Box::from_raw(result) }; // makes sure memory is dropped
                let token_str = unsafe { CStr::from_ptr(boxed.token).to_str().unwrap().to_owned() };
                unsafe {
                    if !boxed.token.is_null() {
                        deallocate_vec(boxed.token as *mut u8, 37);
                    }
                }

                let checkpoint = CheckPoint {
                    checked: boxed.checked,
                    token: token_str,
                };
                Ok(checkpoint)
            }
        }
    }

    pub fn checkpoint_index(&self) -> Result<CheckPoint, FasterError<'_>> {
        if self.storage_dir.is_none() {
            return Err(FasterError::InvalidType);
        }

        let result = unsafe { ffi::faster_checkpoint_index(self.faster_t) };
        match result.is_null() {
            true => Err(FasterError::CheckpointError),
            false => {
                let boxed = unsafe { Box::from_raw(result) }; // makes sure memory is dropped
                let token_str = unsafe { CStr::from_ptr(boxed.token).to_str().unwrap().to_owned() };
                unsafe {
                    if !boxed.token.is_null() {
                        deallocate_vec(boxed.token as *mut u8, 37);
                    }
                }

                let checkpoint = CheckPoint {
                    checked: boxed.checked,
                    token: token_str,
                };
                Ok(checkpoint)
            }
        }
    }

    pub fn checkpoint_hybrid_log(&self) -> Result<CheckPoint, FasterError<'_>> {
        if self.storage_dir.is_none() {
            return Err(FasterError::InvalidType);
        }

        let result = unsafe { ffi::faster_checkpoint_hybrid_log(self.faster_t) };
        match result.is_null() {
            true => Err(FasterError::CheckpointError),
            false => {
                let boxed = unsafe { Box::from_raw(result) }; // makes sure memory is dropped
                let token_str = unsafe { CStr::from_ptr(boxed.token).to_str().unwrap().to_owned() };
                unsafe {
                    if !boxed.token.is_null() {
                        deallocate_vec(boxed.token as *mut u8, 37);
                    }
                }

                let checkpoint = CheckPoint {
                    checked: boxed.checked,
                    token: token_str,
                };
                Ok(checkpoint)
            }
        }
    }

    pub fn recover(
        &self,
        index_token: String,
        hybrid_log_token: String,
    ) -> Result<Recover, FasterError<'_>> {
        if self.storage_dir.is_none() {
            return Err(FasterError::InvalidType);
        }
        let index_token_c = CString::new(index_token).unwrap();
        let index_token_ptr = index_token_c.into_raw();

        let hybrid_token_c = CString::new(hybrid_log_token).unwrap();
        let hybrid_token_ptr = hybrid_token_c.into_raw();

        let recover_result = unsafe {
            let rec = ffi::faster_recover(self.faster_t, index_token_ptr, hybrid_token_ptr);
            let _ = CString::from_raw(index_token_ptr);
            let _ = CString::from_raw(hybrid_token_ptr);
            rec
        };

        match recover_result.is_null() {
            true => Err(FasterError::RecoveryError),
            false => {
                let boxed = unsafe { Box::from_raw(recover_result) }; // makes sure mem is freed
                let sessions_count = boxed.session_ids_count;
                let session_ids_len = sessions_count as usize * 37;
                let mut session_ids_vec: Vec<String> = Vec::new();
                for i in 0..sessions_count {
                    let id = unsafe {
                        CStr::from_ptr((boxed.session_ids).offset(37 * i as isize))
                            .to_str()
                            .unwrap()
                            .to_owned()
                    };
                    session_ids_vec.push(id);
                }
                unsafe {
                    if !boxed.session_ids.is_null() {
                        deallocate_vec(boxed.session_ids as *mut u8, session_ids_len as u64);
                    }
                }
                let recover = Recover {
                    status: boxed.status,
                    version: boxed.version,
                    session_ids: session_ids_vec,
                };
                Ok(recover)
            }
        }
    }

    pub fn complete_pending(&self, b: bool) {
        unsafe { ffi::faster_complete_pending(self.faster_t, b) }
    }

    pub fn start_session(&self) -> String {
        unsafe {
            let c_guid = ffi::faster_start_session(self.faster_t);
            let rust_str = CStr::from_ptr(c_guid).to_str().unwrap().to_owned();
            ffi::faster_free_session(c_guid);
            rust_str
        }
    }

    pub fn continue_session(&self, token: String) -> u64 {
        let token_str = CString::new(token).unwrap();
        let token_ptr = token_str.into_raw();
        unsafe {
            let result = ffi::faster_continue_session(self.faster_t, token_ptr);
            let _ = CString::from_raw(token_ptr);
            result
        }
    }

    pub fn stop_session(&self) {
        unsafe { ffi::faster_stop_session(self.faster_t) }
    }

    /// Advance the epoch for this thread's active session.
    ///
    /// FASTER requires periodic refresh calls from each thread that has an active session. If
    /// refresh calls are missing, safe addresses can stall which can cause auto-compaction to make
    /// little progress and, once the log hits its size budget, force foreground operations to
    /// participate in compaction.
    ///
    /// See [Configuring the Hybrid Log](https://microsoft.github.io/FASTER/docs/fasterkv-tuning/#configuring-the-hybrid-log).
    pub fn refresh(&self) {
        unsafe {
            ffi::faster_refresh_session(self.faster_t);
        }
    }

    pub fn refresh_if_due(&self, last: &mut Instant, every: Duration) {
        if last.elapsed() >= every {
            self.refresh();
            *last = Instant::now();
        }
    }

    pub fn dump_distribution(&self) {
        unsafe {
            ffi::faster_dump_distribution(self.faster_t);
        }
    }

    pub fn grow_index(&self) -> bool {
        unsafe { ffi::faster_grow_index(self.faster_t) }
    }

    // Warning: Calling this will remove the stored data
    pub fn clean_storage(&self) -> Result<(), FasterError<'_>> {
        match &self.storage_dir {
            None => Err(FasterError::InvalidType),
            Some(dir) => {
                fs::remove_dir_all(dir)?;
                Ok(())
            }
        }
    }

    fn destroy(&self) {
        unsafe {
            ffi::faster_destroy(self.faster_t);
        }
    }
}

impl Default for FasterKv {
    fn default() -> Self {
        FasterKvConfig::builder()
            .table_size(1 << 15)
            .log_size(1024 * 1024 * 1024)
            .build()
            .unwrap()
    }
}

// In order to make sure we release the resources the C interface has allocated for the store
impl Drop for FasterKv {
    fn drop(&mut self) {
        self.destroy();
    }
}

unsafe impl Send for FasterKv {}
unsafe impl Sync for FasterKv {}
