//! Thread local runtime context
use crate::runtime::{Handle, TryCurrentError};

use std::cell::RefCell;
use std::ffi::c_void;
use std::thread::LocalKey;


thread_local! {
    static CONTEXT: RefCell<Option<Handle>> = const { RefCell::new(None) }
}

static mut CONTEXT_PTR : &LocalKey<RefCell<Option<Handle>>> = &CONTEXT;

/// Get a ptr of the current TLS context (dynamic library use case).
/// This opaque ptr must be use with the [`set_context_ptr`] in an init function of
/// the dynamic library before it can use any tokio functionality.
#[allow(dead_code)]
pub fn context_ptr() -> *const c_void
{
    &CONTEXT as *const LocalKey<RefCell<Option<Handle>>> as *const c_void
}

/// Set the context of the main process (dynamic library use case).
/// Must be called from the dynamically loaded library with the context ptr created with
/// [`context_ptr`] from the main process (the loader)
#[allow(dead_code)]
pub fn set_context_ptr(ptr: *const c_void)
{
    unsafe { CONTEXT_PTR = &*(ptr as *const LocalKey<RefCell<Option<Handle>>>) };
}

macro_rules! try_with
{
    ($f: expr) => { unsafe { CONTEXT_PTR.try_with($f) } }
}
macro_rules! with
{
    ($f: expr) => { unsafe { CONTEXT_PTR.with($f) } }
}


pub(crate) fn try_current() -> Result<Handle, crate::runtime::TryCurrentError> {
    match try_with!(|ctx| ctx.borrow().clone()) {
        Ok(Some(handle)) => Ok(handle),
        Ok(None) => Err(TryCurrentError::new_no_context()),
        Err(_access_error) => Err(TryCurrentError::new_thread_local_destroyed()),
    }
}

#[track_caller]
pub(crate) fn current() -> Handle {
    match try_current() {
        Ok(handle) => handle,
        Err(e) => panic!("{}", e),
    }
}

cfg_io_driver! {
    #[track_caller]
    pub(crate) fn io_handle() -> crate::runtime::driver::IoHandle {
        match try_with!(|ctx| {
            let ctx = ctx.borrow();
            ctx.as_ref().expect(crate::util::error::CONTEXT_MISSING_ERROR).as_inner().io_handle.clone()
        }) {
            Ok(io_handle) => io_handle,
            Err(_) => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
        }
    }
}

cfg_signal_internal! {
    #[cfg(unix)]
    pub(crate) fn signal_handle() -> crate::runtime::driver::SignalHandle {
        match try_with!(|ctx| {
            let ctx = ctx.borrow();
            ctx.as_ref().expect(crate::util::error::CONTEXT_MISSING_ERROR).as_inner().signal_handle.clone()
        }) {
            Ok(signal_handle) => signal_handle,
            Err(_) => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
        }
    }
}

cfg_time! {
    pub(crate) fn time_handle() -> crate::runtime::driver::TimeHandle {
        match try_with!(|ctx| {
            let ctx = ctx.borrow();
            ctx.as_ref().expect(crate::util::error::CONTEXT_MISSING_ERROR).as_inner().time_handle.clone()
        }) {
            Ok(time_handle) => time_handle,
            Err(_) => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
        }
    }

    cfg_test_util! {
        pub(crate) fn clock() -> Option<crate::runtime::driver::Clock> {
            match try_with!(|ctx| (*ctx.borrow()).as_ref().map(|ctx| ctx.as_inner().clock.clone())) {
                Ok(clock) => clock,
                Err(_) => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
            }
        }
    }
}

cfg_rt! {
    pub(crate) fn spawn_handle() -> Option<crate::runtime::Spawner> {
        match try_with!(|ctx| (*ctx.borrow()).as_ref().map(|ctx| ctx.spawner.clone())) {
            Ok(spawner) => spawner,
            Err(_) => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
        }
    }
}

/// Sets this [`Handle`] as the current active [`Handle`].
///
/// [`Handle`]: Handle
pub(crate) fn enter(new: Handle) -> EnterGuard {
    match try_enter(new) {
        Some(guard) => guard,
        None => panic!("{}", crate::util::error::THREAD_LOCAL_DESTROYED_ERROR),
    }
}

/// Sets this [`Handle`] as the current active [`Handle`].
///
/// [`Handle`]: Handle
pub(crate) fn try_enter(new: Handle) -> Option<EnterGuard> {
    try_with!(|ctx| {
            let old = ctx.borrow_mut().replace(new);
            EnterGuard(old)
        })
        .ok()
}

#[derive(Debug)]
#[repr(C)]
pub(crate) struct EnterGuard(Option<Handle>);

impl Drop for EnterGuard {
    fn drop(&mut self) {
        with!(|ctx| {
            *ctx.borrow_mut() = self.0.take();
        });
    }
}
